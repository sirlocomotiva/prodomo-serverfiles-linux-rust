//! Putting an item on and taking it off: `EquipItem`, `UnequipItem`, `SwapItem`, and the
//! `EquipTo`, `Unequip` and `AddToCharacter` steps under them (`G/char_item.cpp:8164-8600`,
//! `G/item.cpp:437-530`, `:1402-1597`).
//!
//! [`move_item`](crate::character::move_item) routes here when a `CG_ITEM_MOVE` names a wear
//! cell and the caller hands it the character's [`Gear`]. Each step appends what legacy sends
//! to [`Trail`]: the item records, the points records `ApplyPoint` and the computations write,
//! the `GC_CHARACTER_UPDATE` `UpdatePacket` writes (a [`CharacterLook`]), the special effects
//! and the chat notices. A step that fails with nothing appended changed nothing; one that
//! fails after a change leaves the change, as legacy's does.
//!
//! # What is refused as not ported
//!
//! Before its first change, an equip refuses an item whose wearing starts a system this build
//! does not have ([`WornSystem`]): a dragon soul stone or an aura costume, an
//! `ITEM_UNIQUE` or `WEARABLE_UNIQUE` item other than the hide-alignment cloak 70048, a
//! timed item, an accessory with stones, an item with immunities, and a bonus type whose
//! point is not ported. The item load refuses the same items in a wear cell, so everything
//! worn can be taken off.
//!
//! # One Defect this does not reproduce
//!
//! `SwapItem` takes the worn item off before `EquipTo` and `AddToCharacter` run, and neither
//! reports a failure it can meet (`G/char_item.cpp:8237-8250`). Here every check runs first,
//! so a swap either happens whole or changes nothing.
//!
//! The riding gates read `IsRiding` (the horse ridden or a mount vnum) and the polymorph gate
//! the points' polymorph race. Unreachable here, and so not written: the ride item's wedding
//! check (a unique, which is not ported, refuses later), the unique unstack, the
//! wedding checks `__FIX_COSTUM_NUNTA_PESTE_COSTUM_NORMAL__` repeats (the `ENABLE_WEDDING_FIX`
//! ones return first), `REAL_TIME_FIRST_USE`, the mount quest and the aura window; the lock
//! (`IsSecured`) gates the equip and the unequip at legacy's places, with the popup the
//! game state chats. `EquipItem`'s `IsExchanging` (`:8315`) stays out: the move and the
//! use refuse an offered item before they get here. The plain equip syncs the quickslots
//! (`:8525-8531`); the swap and the take-off sync nothing, as legacy's do.

use common::enums::EWearPositions;
use common::item_slots::{
    EWindows, BELT_INVENTORY_SLOT_END, BELT_INVENTORY_SLOT_START, INVENTORY_MAX_NUM, WEAR_MAX_NUM,
};
use common::point_slot as point;
use gamedata::item_custom_category::{is_custom_category, CATEGORY_NUM};
use gamedata::item_kind::{
    ARMOR_BODY, COSTUME_AURA, COSTUME_BODY, COSTUME_HAIR, COSTUME_MOUNT, COSTUME_PET, COSTUME_SASH,
    COSTUME_WEAPON, ITEM_ANTIFLAG_ASSASSIN, ITEM_ANTIFLAG_FEMALE, ITEM_ANTIFLAG_MALE,
    ITEM_ANTIFLAG_SHAMAN, ITEM_ANTIFLAG_SURA, ITEM_ANTIFLAG_WARRIOR, ITEM_ARMOR, ITEM_BELT,
    ITEM_COSTUME, ITEM_DS, ITEM_PICK, ITEM_POLYMORPH, ITEM_RING, ITEM_ROD, ITEM_SKILLBOOK,
    ITEM_SPECIAL_DS, ITEM_TALISMAN, ITEM_TOTEM, ITEM_UNIQUE, ITEM_USE, ITEM_WEAPON, LIMIT_CHAMPION,
    LIMIT_CON, LIMIT_DEX, LIMIT_INT, LIMIT_LEVEL, LIMIT_STR, USE_SPECIAL, WEARABLE_ABILITY,
    WEARABLE_ARROW, WEARABLE_BODY, WEARABLE_DARK, WEARABLE_EAR, WEARABLE_EARTH, WEARABLE_ELEC,
    WEARABLE_FIRE, WEARABLE_FOOTS, WEARABLE_GLOVE, WEARABLE_HEAD, WEARABLE_ICE, WEARABLE_NECK,
    WEARABLE_SHIELD, WEARABLE_UNIQUE, WEARABLE_WEAPON, WEARABLE_WIND, WEARABLE_WRIST,
};
use gamedata::item_proto::{ItemProto, ItemProtos};
use protocol::item_pos::ItemPos;

use super::dice::{number, Dice};
use super::equipment::{
    accessory_socket_grade, counted_attrs, is_energy_wear, is_set_item, item_applies,
    removal_applies, Equipment, Worn, PARTS,
};
use super::item_move::{
    ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, MoveRefused, Unported,
};
use super::items::CharacterItems;
use super::points::{apply_is_ported, race_to_job, Points};
use crate::item::{gc_item_clear, Item, ITEM_FLAG_IRREMOVABLE};

/// `SE_EQUIP_RAMADAN_RING` and the three other ring and pendant effects, by vnum
/// (`G/char_item.cpp:8560-8578`).
const VNUM_EFFECTS: [(u32, u8); 4] = [(71_135, 21), (71_136, 22), (71_143, 23), (71_145, 24)];

/// `SE_EFFECT_SASH_EQUIP`.
const SASH_EQUIP_EFFECT: u8 = 26;

/// The wedding dress and suits, 11901-11904.
const WEDDING_ARMOURS: core::ops::RangeInclusive<u32> = 11_901..=11_904;

/// `SASH_ABSORPTION_SOCKET`.
const SASH_ABSORPTION_SOCKET: usize = 0;

/// What wearing and taking off read and change beyond the item storage.
pub struct Gear<'a> {
    /// The character's points, which the applies and computations change.
    pub points: &'a mut Points,
    /// The prototypes.
    pub protos: &'a ItemProtos,
    /// The special item groups, which a worn unique or ring's applies and effect read.
    pub groups: &'a gamedata::special_item_group::SpecialItemGroups,
    /// The draw a sash's absorption is rolled from.
    pub dice: &'a mut dyn Dice,
    /// Whether the character attacked or used a skill in the last 1.5 s
    /// (`G/char_item.cpp:8471-8477`).
    pub recently_fought: bool,
    /// The `get_global_time()` second a first-use item's clock starts from when used
    /// (`G/char_item.cpp:2815`): `None` keeps the old refusal, so a caller without a clock
    /// stamps nothing.
    pub first_use_now: Option<u32>,
}

/// The sash half of `CItem::AddToCharacter` (`G/item.cpp:487-516`): a sash whose absorption
/// is unset rolls it from its grade. Answers whether it rolled.
pub(crate) fn roll_sash(item: &mut Item, proto: &ItemProto, dice: &mut dyn Dice) -> bool {
    let sash = proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_SASH;
    let rolled = sash && item.sockets[SASH_ABSORPTION_SOCKET] == 0;
    if rolled {
        item.sockets[SASH_ABSORPTION_SOCKET] = match proto.values[0] {
            2 => 5,
            3 => 10,
            4 => number(dice, 11, 19),
            _ => 1,
        };
    }
    rolled
}

/// The unique and ring tail of the equip effect switch
/// (`G/char_item.cpp:8592-8604`): the 128 raw path bytes of the attr group the worn
/// item's `SIGVnum` answers for its own vnum (`GetAttrVnum(item->GetVnum())`, the
/// masked one), zero-padded as legacy's `memcpy` leaves the packet. `None` when the
/// item carries no `SIGVnum`, is not a unique or a ring, or its group, row or attr
/// group went missing. A path past 128 bytes is cut the way the call cuts it.
fn sig_effect_path(
    item: &Item,
    proto: &ItemProto,
    groups: &gamedata::special_item_group::SpecialItemGroups,
) -> Option<[u8; 128]> {
    if item.sig_vnum == 0 || (proto.item_type != ITEM_UNIQUE && proto.item_type != ITEM_RING) {
        return None;
    }
    let key = i32::from_ne_bytes(item.sig_vnum.to_ne_bytes());
    let bag = groups.special.get(&key)?;
    let attr = i32::from_ne_bytes(bag.attr_vnum(item.vnum).to_ne_bytes());
    let group = groups.attr.get(&attr)?;
    let mut path = [0u8; 128];
    let len = group.effect_file.len().min(path.len());
    path[..len].copy_from_slice(&group.effect_file[..len]);
    Some(path)
}

impl core::fmt::Debug for Gear<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Gear")
            .field("recently_fought", &self.recently_fought)
            .field("first_use_now", &self.first_use_now)
            .finish_non_exhaustive()
    }
}

/// What `UpdatePacket` sends of a character whose look changed (`G/char.cpp:1277-1340`).
///
/// The other fields of `GC_CHARACTER_UPDATE` (the state flags, the affects, the guild, the
/// alignment, the PK mode and the premium) come from systems this build does not
/// have, and are sent as 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterLook {
    /// The parts, in `EParts` order.
    pub parts: [u16; PARTS],
    /// `GetLimitPoint(POINT_MOV_SPEED)`, as the `BYTE` the packet keeps.
    pub moving_speed: u8,
    /// `GetLimitPoint(POINT_ATT_SPEED)`, as the `BYTE` the packet keeps.
    pub attack_speed: u8,
    /// The level.
    pub level: u32,
    /// The conqueror level.
    pub conqueror_level: u32,
    /// `GetRefineElementType`.
    pub refine_element_type: u8,
    /// `GetMountVnum` (`G/char.cpp:1354`): the mounted vnum, or 0.
    pub mount_vnum: u32,
}

/// A system that wearing an item starts, which this build has not ported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WornSystem {
    /// A dragon soul stone: `EquipTo` activates it in the dragon soul deck.
    DragonSoul,
    /// An aura costume: its drain, its armour and its booster timer.
    Aura,
    /// An `ITEM_UNIQUE` or `WEARABLE_UNIQUE` item: its expiry timer, its special group and the
    /// alignment title it can hide.
    Unique,
    /// An item whose time runs: a real-time limit, a real-time limit from first use, or a
    /// timer that runs while it is worn.
    Timer,
    /// An accessory with stones in its sockets, which lose one on a timer while it is worn.
    AccessoryTimer,
    /// A bonus type whose point is not ported ([`apply_is_ported`]).
    Apply(u8),
    /// A prototype with immunity flags, which the Rewrite does not grant (see [`Points`]).
    Immunity,
}

impl core::fmt::Display for WornSystem {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DragonSoul => formatter.write_str("the dragon soul deck"),
            Self::Aura => formatter.write_str("the aura costume"),
            Self::Unique => formatter.write_str("the unique item expiry"),
            Self::Timer => formatter.write_str("the item timers"),
            Self::AccessoryTimer => formatter.write_str("the accessory stone timer"),
            Self::Apply(apply) => write!(formatter, "the bonus type {apply}"),
            Self::Immunity => formatter.write_str("the item immunities"),
        }
    }
}

/// The first system `EquipTo` would start for this item that the Rewrite has not ported.
///
/// `EquipTo` activates a dragon soul stone, applies the item's bonuses (`ModifyPoints`), starts
/// the unique, wear-timer, accessory and aura-booster timers (`G/item.cpp:1458-1487`);
/// the mount and pet costumes' summons are the world's (`MountSummon`, `PetSummon`);
/// `OnAfterCreatedItem` starts the real-time timer (`:2775-2781`).
/// `BuffOnAttr_AddBuffsFromItem` runs beside the wear
/// ([`Points::energy_buff_add`](super::points::Points::energy_buff_add)): only the costume-attr
/// pool stays refused, because nothing on the owner's data fills it.
///
/// A plain `LIMIT_REAL_TIME` item starts no equip timer (`G/item.cpp:1458-1487` names only the
/// unique, wear, accessory and booster starts, and `IsRealTimeItem` exempts the wear one):
/// its clock runs from creation whether worn or not, and the world's expiry tick removes it.
/// Only an unstamped first-use clock (the use stamps, not the wear, so a use-to-wear lands
/// stamped), a wear timer (all owner's rows are dragon soul, which never reaches here) and
/// the accessory stones refuse.
#[must_use]
pub fn worn_system_not_ported(
    item: &Item,
    proto: &ItemProto,
    protos: &ItemProtos,
    groups: &gamedata::special_item_group::SpecialItemGroups,
) -> Option<WornSystem> {
    if proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_AURA {
        return Some(WornSystem::Aura);
    }
    // An `ITEM_UNIQUE` starts the unique expiry event (`EquipTo`, `G/item.cpp:1458-1487`,
    // `StartUniqueExpireEvent`, `G/item.cpp:2165-2194`), which the game state's wear sync
    // arms. Its applies run through `ModifyPoints`'s unique and ring tail (`sig_applies`),
    // and the tail's apply gate below refuses what is not ported — so a unique wears
    // unless it is real-time (its expiry is another event) or carries a special group
    // the owner's data never fills for one (every group vnum sits above the 30000 gate,
    // `G/item_manager_read_tables.cpp:278-282`).
    if proto.item_type == ITEM_UNIQUE
        && (proto
            .limits
            .iter()
            .any(|limit| limit.kind == gamedata::item_kind::LIMIT_REAL_TIME)
            || item.sig_vnum != 0)
    {
        return Some(WornSystem::Unique);
    }
    if proto.timer_based_on_wear.is_some() {
        return Some(WornSystem::Timer);
    }
    // A started first-use clock wears: the use stamped socket 1 before the equip checks
    // ran (`UseItemEx`, `G/char_item.cpp:2804-2820`), so only an unstamped one refuses.
    if proto.real_time_first_use.is_some() && item.sockets[USE_SOCKET] == 0 {
        return Some(WornSystem::Timer);
    }
    if accessory_socket_grade(item, proto) > 0 {
        return Some(WornSystem::AccessoryTimer);
    }
    if proto.immune_flags != 0 {
        return Some(WornSystem::Immunity);
    }
    item_applies(item, proto, protos, groups)
        .into_iter()
        .find(|(apply, _)| !apply_is_ported(*apply))
        .map(|(apply, _)| WornSystem::Apply(apply))
}

/// The anti-flag that refuses a race's job, or 0 for a job with none (`CItem::CanUsedBy`,
/// `G/item.cpp:599-625`, and the same switch in `CanEquipNow`).
pub(crate) fn job_anti_flag(race: u8) -> u32 {
    [
        ITEM_ANTIFLAG_WARRIOR,
        ITEM_ANTIFLAG_ASSASSIN,
        ITEM_ANTIFLAG_SURA,
        ITEM_ANTIFLAG_SHAMAN,
    ]
    .get(usize::from(race_to_job(race)))
    .copied()
    .unwrap_or(0)
}

/// The anti-flag that refuses a race's sex (`FN_check_item_sex`, `G/char_item.cpp:191-207`).
pub(crate) fn sex_anti_flag(race: u8) -> u32 {
    if is_female(race) {
        ITEM_ANTIFLAG_FEMALE
    } else {
        ITEM_ANTIFLAG_MALE
    }
}

/// `GET_SEX`: whether a race is female. Races 1, 3, 4 and 6 are; every other race is male.
#[must_use]
pub fn is_female(race: u8) -> bool {
    matches!(race, 1 | 3 | 4 | 6)
}

/// `CItem::IsEquipable` (`G/item.cpp:1378-1400`).
#[must_use]
pub fn is_equipable(proto: &ItemProto) -> bool {
    [
        ITEM_COSTUME,
        ITEM_ARMOR,
        ITEM_WEAPON,
        ITEM_ROD,
        ITEM_PICK,
        ITEM_UNIQUE,
        ITEM_DS,
        ITEM_SPECIAL_DS,
        ITEM_RING,
        ITEM_BELT,
        ITEM_TALISMAN,
    ]
    .contains(&proto.item_type)
}

/// The flat cell of a wear cell: `INVENTORY_MAX_NUM + wear`.
#[must_use]
pub fn wear_pos(wear: u16) -> ItemPos {
    ItemPos::new(EWindows::Inventory as u8, INVENTORY_MAX_NUM + wear)
}

/// `CItem::IsRideItem` (`G/item.cpp:2389-2400`, with `ENABLE_MOUNT_COSTUME_SYSTEM`): a
/// unique of `UNIQUE_SPECIAL_RIDE` (2) or `UNIQUE_SPECIAL_MOUNT_RIDE` (3), or a mount
/// costume.
#[must_use]
pub fn is_ride_item(proto: &ItemProto) -> bool {
    (proto.item_type == ITEM_UNIQUE && matches!(proto.sub_type, 2 | 3))
        || (proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_MOUNT)
}

/// The item a wear cell holds.
fn worn_at(items: &CharacterItems, wear: u16) -> Option<Item> {
    items.item_at(wear_pos(wear)).cloned()
}

/// The wear cell a position names, or `None` when it names another cell.
///
/// The equipment is the flat cells from [`INVENTORY_MAX_NUM`] to
/// `INVENTORY_MAX_NUM + WEAR_MAX_NUM` (`common/src/item_slots.rs`), which the load and the
/// equip path both address through the inventory window (`prodomo/src/item_load.rs`).
#[must_use]
pub fn wear_of(pos: ItemPos) -> Option<u16> {
    if pos.window_type != EWindows::Inventory as u8 && pos.window_type != EWindows::Equipment as u8
    {
        return None;
    }
    pos.cell
        .checked_sub(INVENTORY_MAX_NUM)
        .filter(|wear| *wear < WEAR_MAX_NUM)
}

/// The points a worn item gave go with it (`CItem::Unequip`, `G/item.cpp:1544`).
///
/// The death penalty takes a worn item for the ground without the equip path
/// (`G/char_battle.cpp:1114-1153`), but the points go as they do in legacy's
/// `RemoveFromCharacter`, which calls `Unequip` (`G/item.cpp:369-377`). The records are the
/// point changes and the look, which the victim's client is sent.
///
/// The item itself is not released here: legacy's `Unequip` empties the wear cell before the
/// computation that follows it, so the caller releases the item between this and
/// [`recompute_points`].
///
/// # Errors
///
/// [`MoveRefused::UnknownVnum`] when the item has no prototype.
pub fn unequip_points(
    gear: &mut Gear<'_>,
    items: &mut CharacterItems,
    wear: u16,
    item: &Item,
) -> Result<Vec<MoveRecord>, MoveRefused> {
    let mut equipper = Equipper::new(items, gear, 0);
    let proto = equipper.proto(item.vnum)?;
    equipper.modify_points(wear, item, proto, false);
    Ok(std::mem::take(&mut equipper.trail.records))
}

/// The points are computed again after a worn item left (`ComputeBattlePoints` and
/// `ComputePoints`, `G/item.cpp:1580-1587`).
///
/// The records are the point changes and the look, which the victim's client is sent.
pub fn recompute_points(
    gear: &mut Gear<'_>,
    items: &mut CharacterItems,
    vnum: u32,
) -> Vec<MoveRecord> {
    let mut equipper = Equipper::new(items, gear, 0);
    equipper.recompute(vnum);
    std::mem::take(&mut equipper.trail.records)
}
/// `do_item_full_set`'s unequip and equip (`G/cmd_gm.cpp:4760-4902`): the six first cells
/// and the shield come off first (`UnequipItem`), then each `vnums` piece is made fresh
/// and goes on where [`find_equip_cell`] says, in order, skipping what cannot go on.
/// `make` hands a fresh [`Item`](crate::item::Item) for a vnum, or `None` when the vnum
/// has no prototype or no id is left; the unequips ran before the first make, so a
/// piece never overwrites what was worn.
pub fn wear_full_set(
    items: &mut CharacterItems,
    gear: &mut Gear<'_>,
    usable_cells: u16,
    vnums: &[u32],
    mut make: impl FnMut(u32) -> Option<Item>,
) -> MoveDone {
    const STRIPPED: [EWearPositions; 7] = [
        EWearPositions::Body,
        EWearPositions::Head,
        EWearPositions::Foots,
        EWearPositions::Wrist,
        EWearPositions::Weapon,
        EWearPositions::Neck,
        EWearPositions::Shield,
    ];
    let mut equipper = Equipper::new(items, gear, usable_cells);
    for wear in STRIPPED {
        if let Some(held) = worn_at(equipper.items(), wear as u16) {
            let _ = equipper.unequip_item(&held);
        }
    }
    for vnum in vnums {
        let Some(piece) = make(*vnum) else { continue };
        let Ok(proto) = equipper.proto(piece.vnum) else {
            continue;
        };
        let Some(wear) = find_equip_cell(equipper.items(), proto) else {
            continue;
        };
        if worn_at(equipper.items(), wear).is_some() {
            continue;
        }
        let _ = equipper.equip_bare(wear, &piece);
    }
    let trail = equipper.into_trail();
    MoveDone {
        kind: MoveKind::FullSet,
        records: trail.records,
        changes: trail.changes,
    }
}

/// `CItem::FindEquipCell` with no candidate (`G/item.cpp:628-779`): the wear cell the item
/// goes to, or `None`. A dragon soul stone answers its deck cell, 64 plus its subtype.
#[must_use]
pub fn find_equip_cell(items: &CharacterItems, proto: &ItemProto) -> Option<u16> {
    let kind = proto.item_type;
    let flags = proto.wear_flags;
    let ungated = [ITEM_COSTUME, ITEM_DS, ITEM_SPECIAL_DS, ITEM_RING, ITEM_BELT].contains(&kind);
    if (flags == 0 || kind == ITEM_TOTEM) && !ungated {
        return None;
    }
    let worn = |wear: EWearPositions| items.item_at(wear_pos(wear as u16)).is_some();
    let cell = match kind {
        ITEM_DS | ITEM_SPECIAL_DS => {
            return u16::try_from(proto.sub_type)
                .ok()
                .map(|sub| common::item_slots::WEAR_MAX_NUM + sub);
        }
        ITEM_TALISMAN => [
            (WEARABLE_FIRE, EWearPositions::TalismanFire),
            (WEARABLE_ICE, EWearPositions::TalismanIce),
            (WEARABLE_EARTH, EWearPositions::TalismanEarth),
            (WEARABLE_DARK, EWearPositions::TalismanDark),
            (WEARABLE_WIND, EWearPositions::TalismanWind),
            (WEARABLE_ELEC, EWearPositions::TalismanElec),
        ]
        .into_iter()
        .find(|(flag, _)| flags & flag != 0)
        .map(|(_, wear)| wear),
        ITEM_COSTUME => match proto.sub_type {
            COSTUME_BODY => Some(EWearPositions::CostumeBody),
            COSTUME_HAIR => Some(EWearPositions::CostumeHair),
            COSTUME_MOUNT => Some(EWearPositions::CostumeMount),
            COSTUME_SASH => Some(EWearPositions::CostumeSash),
            COSTUME_WEAPON => Some(EWearPositions::CostumeWeapon),
            COSTUME_AURA => Some(EWearPositions::CostumeAura),
            COSTUME_PET => Some(EWearPositions::CostumePet),
            _ => None,
        },
        ITEM_RING => Some(if worn(EWearPositions::Ring1) {
            EWearPositions::Ring2
        } else {
            EWearPositions::Ring1
        }),
        ITEM_BELT => Some(EWearPositions::Belt),
        _ => return by_wear_flag(items, flags),
    };
    cell.map(|wear| wear as u16)
}

/// The wear-flag arms of `FindEquipCell`, in legacy's order.
fn by_wear_flag(items: &CharacterItems, flags: u32) -> Option<u16> {
    let worn = |wear: u16| items.item_at(wear_pos(wear)).is_some();
    let single = [
        (WEARABLE_BODY, EWearPositions::Body),
        (WEARABLE_HEAD, EWearPositions::Head),
        (WEARABLE_FOOTS, EWearPositions::Foots),
        (WEARABLE_WRIST, EWearPositions::Wrist),
        (WEARABLE_WEAPON, EWearPositions::Weapon),
        (WEARABLE_SHIELD, EWearPositions::Shield),
        (WEARABLE_NECK, EWearPositions::Neck),
        (WEARABLE_EAR, EWearPositions::Ear),
        (WEARABLE_ARROW, EWearPositions::Arrow),
        (WEARABLE_GLOVE, EWearPositions::Glove),
    ];
    if let Some((_, wear)) = single.into_iter().find(|(flag, _)| flags & flag != 0) {
        return Some(wear as u16);
    }
    if flags & WEARABLE_UNIQUE != 0 {
        let first = EWearPositions::Unique1 as u16;
        return Some(if worn(first) {
            EWearPositions::Unique2 as u16
        } else {
            first
        });
    }
    if flags & WEARABLE_ABILITY != 0 {
        let first = EWearPositions::Ability1 as u16;
        return (first..first + 8).find(|&wear| !worn(wear));
    }
    None
}

/// What the equip steps have sent and changed so far, in order.
#[derive(Debug, Default)]
pub(crate) struct Trail {
    /// What the client is sent.
    pub(crate) records: Vec<MoveRecord>,
    /// What the store has to change.
    pub(crate) changes: Vec<ItemChange>,
}

impl Trail {
    /// Send a refusal's notice, as legacy does at the point it refuses.
    fn notice(&mut self, refused: MoveRefused) {
        if let Some(text) = refused.notice() {
            self.records.push(MoveRecord::Notice(text));
        }
    }
}

/// What `UpdatePacket` sends for a character with `items`, `points` and `protos`: its parts,
/// move and attack speeds, levels, the refine element of its worn gear and its mount. It reads the
/// character only, so the game state can send it without the steps' gear.
pub fn look_of(items: &CharacterItems, points: &Points, protos: &ItemProtos) -> CharacterLook {
    let byte = |kind: usize| u8::try_from(points.limit_point(kind) & 0xff).unwrap_or(0);
    CharacterLook {
        parts: points.parts(),
        moving_speed: byte(point::POINT_MOV_SPEED),
        attack_speed: byte(point::POINT_ATT_SPEED),
        level: u32::from(points.level()),
        conqueror_level: u32::from(points.conqueror_level()),
        refine_element_type: Equipment::of(items, protos).refine_element_type(),
        mount_vnum: points.mount_vnum(),
    }
}

/// The character's storage and gear, with the trail each step appends to.
pub(crate) struct Equipper<'s, 'g> {
    items: &'s mut CharacterItems,
    gear: &'s mut Gear<'g>,
    usable_cells: u16,
    pub(crate) trail: Trail,
}

impl<'s, 'g> Equipper<'s, 'g> {
    /// Steps over `items` and `gear`; `usable_cells` bounds the base-inventory searches.
    pub(crate) fn new(
        items: &'s mut CharacterItems,
        gear: &'s mut Gear<'g>,
        usable_cells: u16,
    ) -> Self {
        Self {
            items,
            gear,
            usable_cells,
            trail: Trail::default(),
        }
    }

    /// The storage.
    pub(crate) fn items(&self) -> &CharacterItems {
        self.items
    }

    /// What the steps sent and changed.
    pub(crate) fn into_trail(self) -> Trail {
        self.trail
    }

    /// The item's prototype.
    pub(crate) fn proto(&self, vnum: u32) -> Result<&'g ItemProto, MoveRefused> {
        let protos: &'g ItemProtos = self.gear.protos;
        protos.get(vnum).ok_or(MoveRefused::UnknownVnum(vnum))
    }

    /// Send a refusal's notice now.
    pub(crate) fn notice(&mut self, refused: MoveRefused) {
        self.trail.notice(refused);
    }

    /// The worn belt's `value0`, or `None` when no belt is worn.
    fn belt_grade(&self) -> Option<i32> {
        let belt = worn_at(self.items, EWearPositions::Belt as u16)?;
        self.gear.protos.get(belt.vnum).map(|proto| proto.values[0])
    }

    /// `IsEmptyItemGrid` for a flat cell of the inventory window.
    pub(crate) fn is_empty_grid(&self, dest: ItemPos, size: u8, exception: Option<u16>) -> bool {
        self.items
            .is_empty_item_grid(dest, size, exception, self.usable_cells, self.belt_grade())
    }

    /// `GetEmptyInventoryEx` for a non-dragon-soul item (`G/char_item.cpp:1202-1256`): the
    /// first of the item's banks with room, else the first free base cell.
    fn empty_inventory_cell(&self, item: &Item, proto: &ItemProto) -> Option<u16> {
        (0..CATEGORY_NUM)
            .filter(|&category| is_custom_category(proto, category))
            .find_map(|category| self.items.find_free_custom_cell(category, item.size))
            .or_else(|| {
                self.items
                    .find_free_inventory_cell(self.usable_cells, item.size)
            })
    }

    /// What `UpdatePacket` sends now.
    pub fn look(&self) -> CharacterLook {
        look_of(self.items, &*self.gear.points, self.gear.protos)
    }

    fn push_points(&mut self, records: Vec<super::points::PointRecord>) {
        self.trail
            .records
            .extend(records.into_iter().map(MoveRecord::Point));
    }

    /// `CanEquipNow` (`G/char_item.cpp:10060-10193`).
    pub(crate) fn can_equip_now(&self, item: &Item, proto: &ItemProto) -> Result<(), MoveRefused> {
        let points = &*self.gear.points;
        if proto.anti_flags & job_anti_flag(points.race()) != 0 {
            return Err(MoveRefused::JobAntiFlag);
        }
        for limit in &proto.limits {
            let (have, refused) = match limit.kind {
                LIMIT_LEVEL => (i32::from(points.level()), MoveRefused::LevelTooLow),
                LIMIT_CHAMPION => (
                    i32::from(points.conqueror_level()),
                    MoveRefused::ChampionTooLow,
                ),
                LIMIT_STR => (points.get_point(point::POINT_ST), MoveRefused::StrTooLow),
                LIMIT_INT => (points.get_point(point::POINT_IQ), MoveRefused::IntTooLow),
                LIMIT_DEX => (points.get_point(point::POINT_DX), MoveRefused::DexTooLow),
                LIMIT_CON => (points.get_point(point::POINT_HT), MoveRefused::ConTooLow),
                _ => continue,
            };
            if have < limit.value {
                return Err(refused);
            }
        }
        // `G/char_item.cpp:10155-10161`, before the same-special-group arm the
        // unique system owns: a marriage unique on an unmarried wearer refuses.
        // No character of this build is married (`sys.marriage` is `unused`).
        if matches!(item.vnum, 71_069..=71_074) {
            return Err(MoveRefused::UnmarriedUnique);
        }
        // A `WEARABLE_UNIQUE` item starts the unique expiry event (`EquipTo`,
        // `G/item.cpp:1458-1487`), which the game state's wear sync arms; the same
        // real-time and special-group uniques refuse here as in `worn_system_not_ported`.
        if proto.wear_flags & WEARABLE_UNIQUE != 0
            && (proto
                .limits
                .iter()
                .any(|limit| limit.kind == gamedata::item_kind::LIMIT_REAL_TIME)
                || item.sig_vnum != 0)
        {
            return Err(MoveRefused::NotPorted(Unported::Worn(WornSystem::Unique)));
        }
        if proto.item_type == ITEM_RING {
            let twice = [EWearPositions::Ring1, EWearPositions::Ring2]
                .into_iter()
                .filter_map(|wear| worn_at(self.items, wear as u16))
                .any(|ring| ring.vnum == item.vnum);
            if twice {
                return Err(MoveRefused::RingTwice);
            }
        }
        // `G/char_item.cpp:10184-10190`, last; the game state chats the popup.
        if self.gear.points.is_secured() {
            return Err(MoveRefused::Secured);
        }
        Ok(())
    }

    /// `CanUnequipNow` (`G/char_item.cpp:10195-10222`).
    pub(crate) fn can_unequip_now(
        &self,
        item: &Item,
        proto: &ItemProto,
    ) -> Result<(), MoveRefused> {
        if proto.item_type == ITEM_BELT {
            let loaded = (BELT_INVENTORY_SLOT_START..BELT_INVENTORY_SLOT_END).any(|cell| {
                self.items
                    .item_at(ItemPos::new(EWindows::Inventory as u8, cell))
                    .is_some()
            });
            if loaded {
                return Err(MoveRefused::BeltNotEmpty);
            }
        }
        if item.flags & ITEM_FLAG_IRREMOVABLE != 0 {
            return Err(MoveRefused::Irremovable);
        }
        if self.empty_inventory_cell(item, proto).is_none() {
            return Err(MoveRefused::NoRoomToUnequip);
        }
        // `G/char_item.cpp:10213-10219`, last; the game state chats the popup.
        if self.gear.points.is_secured() {
            return Err(MoveRefused::Secured);
        }
        Ok(())
    }

    /// The applies, then the parts switch, of `ModifyPoints` (`G/item.cpp:781-1375`).
    fn modify_points(&mut self, wear: u16, item: &Item, proto: &'g ItemProto, add: bool) {
        let applies = item_applies(item, proto, self.gear.protos, self.gear.groups);
        let applies = if add {
            applies
        } else {
            removal_applies(&applies)
        };
        for (apply, value) in applies {
            let records = self.gear.points.apply_point(apply, value);
            self.push_points(records);
        }
        let points = &*self.gear.points;
        let change = Equipment::of(self.items, self.gear.protos).part_change(
            wear,
            Worn { item, proto },
            add,
            u16::from(points.part_base()),
            points.parts(),
        );
        if let Some(change) = change {
            self.gear.points.set_part(change.part, change.value);
            if change.update {
                let look = self.look();
                self.trail.records.push(MoveRecord::Look(look));
            }
        }
    }

    /// The tail of `EquipTo` and `Unequip`: a set piece recomputes the points, anything else
    /// the battle points, and the look is sent either way.
    fn recompute(&mut self, vnum: u32) {
        let equipment = Equipment::of(self.items, self.gear.protos).with_groups(self.gear.groups);
        let records = if is_set_item(vnum) {
            self.gear.points.compute_points_with(&equipment)
        } else {
            self.gear.points.compute_battle_points_with(&equipment)
        };
        self.push_points(records);
        let look = self.look();
        self.trail.records.push(MoveRecord::Look(look));
    }

    /// `CItem::Unequip` (`G/item.cpp:1507-1597`): the item leaves its wear cell and is held by
    /// no cell.
    pub(crate) fn take_off(&mut self, wear: u16, item: &Item) -> Result<(), MoveRefused> {
        let proto = self.proto(item.vnum)?;
        self.modify_points(wear, item, proto, false);
        // `BuffOnAttr_RemoveBuffsFromItem` before the wear clears (`G/item.cpp:1560`).
        if is_energy_wear(wear) {
            let attrs = counted_attrs(item);
            let mut records = Vec::new();
            self.gear.points.energy_buff_remove(&attrs, &mut records);
            self.push_points(records);
        }
        let _ = self.items.release(item.id).map_err(MoveRefused::Storage)?;
        self.trail
            .records
            .push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(wear_pos(
                wear,
            )))));
        self.recompute(item.vnum);
        Ok(())
    }

    /// `RemoveFromCharacter`: a worn item is taken off, anything else cleared from its cell.
    fn remove_from_character(&mut self, item: &Item) -> Result<(), MoveRefused> {
        if item.pos.window_type == EWindows::Equipment as u8 && item.pos.cell >= INVENTORY_MAX_NUM {
            return self.take_off(item.pos.cell - INVENTORY_MAX_NUM, item);
        }
        let pos = self.items.release(item.id).map_err(MoveRefused::Storage)?;
        self.trail
            .records
            .push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
        Ok(())
    }

    /// `CItem::EquipTo` (`G/item.cpp:1402-1505`) for a wear cell the caller found empty.
    fn put_on(&mut self, wear: u16, item: &Item) -> Result<(), MoveRefused> {
        let proto = self.proto(item.vnum)?;
        self.remove_from_character(item)?;
        let pos = wear_pos(wear);
        self.items.set(pos, item).map_err(MoveRefused::Storage)?;
        self.trail
            .records
            .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 0))));
        let stored = self.stored(item)?;
        self.trail.changes.push(ItemChange::Moved {
            id: item.id,
            pos: stored.pos,
        });
        self.modify_points(wear, &stored, proto, true);
        // `BuffOnAttr_AddBuffsFromItem` before the recompute (`G/item.cpp:1477`): a set
        // piece's recompute clears the pool and its affects re-arm it.
        if is_energy_wear(wear) {
            let attrs = counted_attrs(&stored);
            let mut records = Vec::new();
            self.gear.points.energy_buff_add(&attrs, &mut records);
            self.push_points(records);
        }
        self.recompute(item.vnum);
        Ok(())
    }

    fn stored(&self, item: &Item) -> Result<Item, MoveRefused> {
        self.items.item(item.id).cloned().ok_or(MoveRefused::Count(
            super::items::CountRefused::NotHeld(item.id),
        ))
    }

    /// `CItem::AddToCharacter` with no highlight (`G/item.cpp:437-530`): a sash whose
    /// absorption is unset rolls it from its grade first.
    fn add_to_character(&mut self, item: &Item, pos: ItemPos) -> Result<(), MoveRefused> {
        let proto = self.proto(item.vnum)?;
        let mut item = item.clone();
        let rolled = roll_sash(&mut item, proto, self.gear.dice);
        self.items.set(pos, &item).map_err(MoveRefused::Storage)?;
        self.trail
            .records
            .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 0))));
        let stored = self.stored(&item)?;
        self.trail.changes.push(ItemChange::Moved {
            id: item.id,
            pos: stored.pos,
        });
        if rolled {
            self.trail.changes.push(ItemChange::Sockets {
                id: item.id,
                sockets: item.sockets,
            });
        }
        Ok(())
    }

    /// The costume weapon, taken off first when the weapon cell changes. A failure sends the
    /// costume weapon's own notice, and answers `stuck`.
    pub(crate) fn free_costume_weapon(&mut self, stuck: MoveRefused) -> Result<(), MoveRefused> {
        let Some(costume) = worn_at(self.items, EWearPositions::CostumeWeapon as u16) else {
            return Ok(());
        };
        self.unequip_item(&costume).map_err(|refused| {
            self.notice(refused);
            stuck
        })
    }

    /// `CHARACTER::UnequipItem` (`G/char_item.cpp:8266-8311`).
    pub(crate) fn unequip_item(&mut self, item: &Item) -> Result<(), MoveRefused> {
        let proto = self.proto(item.vnum)?;
        if find_equip_cell(self.items, proto) == Some(EWearPositions::Weapon as u16) {
            self.free_costume_weapon(MoveRefused::CostumeWeaponStuck)?;
        }
        self.can_unequip_now(item, proto)?;
        let cell = self
            .empty_inventory_cell(item, proto)
            .ok_or(MoveRefused::NoRoomToUnequip)?;
        let item = self.stored(item)?;
        self.remove_from_character(&item)?;
        self.add_to_character(&item, ItemPos::new(EWindows::Inventory as u8, cell))?;
        let records = self.gear.points.check_maximum_points();
        self.push_points(records);
        Ok(())
    }

    /// The equip-side checks of `EquipItem` that run before its first change, in legacy's
    /// order (`G/char_item.cpp:8313-8479`), with the not-ported refusal after them.
    fn equip_checks(&self, item: &Item, proto: &ItemProto) -> Result<u16, MoveRefused> {
        if !is_equipable(proto) {
            return Err(MoveRefused::NotEquipable);
        }
        self.can_equip_now(item, proto)?;
        let wear = find_equip_cell(self.items, proto).ok_or(MoveRefused::NoEquipCell)?;
        let points = &*self.gear.points;
        let riding = points.horse().riding() || points.mount_vnum() != 0;
        if wear == EWearPositions::Body as u16 && riding && WEDDING_ARMOURS.contains(&item.vnum) {
            return Err(MoveRefused::RiderWeddingArmour);
        }
        if wear != EWearPositions::Arrow as u16 && points.is_polymorphed() {
            return Err(MoveRefused::Polymorphed);
        }
        let armour = worn_at(self.items, EWearPositions::Body as u16);
        let wedding_armour = armour.is_some_and(|armour| WEDDING_ARMOURS.contains(&armour.vnum));
        if wear == EWearPositions::CostumeBody as u16
            && proto.sub_type == COSTUME_BODY
            && wedding_armour
        {
            return Err(MoveRefused::WeddingCostume);
        }
        if worn_at(self.items, EWearPositions::CostumeBody as u16).is_some()
            && WEDDING_ARMOURS.contains(&item.vnum)
        {
            return Err(MoveRefused::WeddingArmour);
        }
        if proto.anti_flags & sex_anti_flag(self.gear.points.race()) != 0 {
            return Err(MoveRefused::WrongSex);
        }
        if riding {
            if is_ride_item(proto) {
                return Err(MoveRefused::RiderRideItem);
            }
            if proto.item_type == ITEM_ARMOR && proto.sub_type == ARMOR_BODY {
                return Err(MoveRefused::RiderArmour);
            }
            if proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_BODY {
                return Err(MoveRefused::RiderCostume);
            }
        }
        if wear != EWearPositions::Arrow as u16 && self.gear.recently_fought {
            return Err(MoveRefused::RecentlyFought);
        }
        if proto.item_type == ITEM_DS || proto.item_type == ITEM_SPECIAL_DS {
            return Err(MoveRefused::NotPorted(Unported::Worn(
                WornSystem::DragonSoul,
            )));
        }
        if let Some(system) =
            worn_system_not_ported(item, proto, self.gear.protos, self.gear.groups)
        {
            return Err(MoveRefused::NotPorted(Unported::Worn(system)));
        }
        Ok(wear)
    }

    /// `CHARACTER::EquipItem` (`G/char_item.cpp:8313-8600`). Answers whether the item went on
    /// through a swap.
    pub(crate) fn equip_item(&mut self, item: &Item) -> Result<bool, MoveRefused> {
        let proto = self.proto(item.vnum)?;
        let wear = self.equip_checks(item, proto)?;
        let armour = worn_at(self.items, EWearPositions::Body as u16);
        if wear == EWearPositions::Weapon as u16 {
            let costume = worn_at(self.items, EWearPositions::CostumeWeapon as u16);
            let mismatched = costume.as_ref().is_some_and(|costume| {
                self.proto(costume.vnum)
                    .is_ok_and(|costume| costume.values[3] != proto.sub_type)
            });
            if proto.item_type != ITEM_WEAPON || mismatched {
                self.free_costume_weapon(MoveRefused::CostumeWeaponStuck)?;
            }
        } else if wear == EWearPositions::CostumeWeapon as u16 && proto.sub_type == COSTUME_WEAPON {
            let weapon = worn_at(self.items, EWearPositions::Weapon as u16)
                .and_then(|weapon| self.proto(weapon.vnum).ok());
            let fits = weapon.is_some_and(|weapon| {
                weapon.item_type == ITEM_WEAPON && proto.values[3] == weapon.sub_type
            });
            if !fits {
                return Err(MoveRefused::WrongWeaponForCostume);
            }
        }
        let item = self.stored(item)?;
        let swapped = match worn_at(self.items, wear) {
            Some(held) if held.flags & ITEM_FLAG_IRREMOVABLE == 0 => {
                if proto.wear_flags == WEARABLE_ABILITY {
                    return Err(MoveRefused::AbilityOccupied);
                }
                self.swap_item(&item, &held, wear)?;
                true
            }
            Some(_) => return Err(MoveRefused::Irremovable),
            None => {
                let from = item.pos.cell;
                self.put_on(wear, &item)?;
                // `SyncQuickslot(QUICKSLOT_TYPE_ITEM, bOldCell, iWearCell)`
                // (`G/char_item.cpp:8525-8531`): the slot is re-pointed at the bare wear
                // index (10 for a shield), which `SetQuickslot` accepts as an inventory
                // cell. The swap arm syncs nothing, and neither does the take-off.
                self.trail.records.push(MoveRecord::QuickslotSync(
                    super::quickslot::QuickslotSync {
                        from,
                        to: super::quickslot::SyncTo::Cell(wear),
                    },
                ));
                false
            }
        };
        let effect = VNUM_EFFECTS
            .iter()
            .find(|(vnum, _)| *vnum == item.vnum)
            .map(|(_, effect)| *effect)
            .or_else(|| {
                let sash = proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_SASH;
                let wedding = armour.is_some_and(|armour| WEDDING_ARMOURS.contains(&armour.vnum));
                (sash && !wedding).then_some(SASH_EQUIP_EFFECT)
            });
        if let Some(effect) = effect {
            self.trail.records.push(MoveRecord::Effect(effect));
        }
        // The unique and ring tail (`G/char_item.cpp:8592-8604`): a worn item with a
        // `SIGVnum` answers its special group's effect path for its own vnum. A group
        // the reload dropped silences it, as does a row or attr group that went missing.
        if let Some(path) = sig_effect_path(&item, proto, self.gear.groups) {
            self.trail.records.push(MoveRecord::SpecificEffect(path));
        }
        Ok(swapped)
    }

    /// `CHARACTER::SwapItem(item.cell, 180 + wear)` (`G/char_item.cpp:8164-8264`): `item`
    /// goes on in place of `held`, which goes to `item`'s cell. Every check runs before the
    /// first change.
    fn swap_item(&mut self, item: &Item, held: &Item, wear: u16) -> Result<(), MoveRefused> {
        let from = item.pos;
        let from_worn = from.cell >= INVENTORY_MAX_NUM
            && from.cell < INVENTORY_MAX_NUM + common::item_slots::WEAR_MAX_NUM;
        if from_worn || from.cell == INVENTORY_MAX_NUM + wear || item.id == held.id {
            return Err(MoveRefused::SwapRefused);
        }
        let cell = ItemPos::new(EWindows::Inventory as u8, from.cell);
        if !self.is_empty_grid(cell, held.size, Some(from.cell)) {
            return Err(MoveRefused::SwapRefused);
        }
        let held_proto = self.proto(held.vnum)?;
        if held_proto.item_type == ITEM_BELT {
            let proto = self.proto(item.vnum)?;
            let checked = self
                .can_unequip_now(held, held_proto)
                .and_then(|()| self.can_equip_now(item, proto));
            if let Err(refused) = checked {
                self.notice(refused);
                return Err(MoveRefused::SwapRefused);
            }
        }
        let proto = self.proto(item.vnum)?;
        if find_equip_cell(self.items, proto) != Some(wear) {
            return Err(MoveRefused::SwapRefused);
        }
        self.take_off(wear, held)?;
        self.put_on(wear, item)?;
        self.add_to_character(held, cell)
    }
    /// `CItem::EquipTo` on a fresh item (`G/item.cpp:1402-1505`), with the seed landed
    /// already: the item goes into `wear` unread, its applies modify the points
    /// (`ModifyPoints(true)`), and the recompute's points records and the look follow.
    /// The caller found `wear` through [`find_equip_cell`] while it stands empty, so
    /// `EquipTo`'s occupied-cell refusal cannot fire.
    fn equip_bare(&mut self, wear: u16, item: &Item) -> Result<(), MoveRefused> {
        let proto = self.proto(item.vnum)?;
        let pos = wear_pos(wear);
        self.items.set(pos, item).map_err(MoveRefused::Storage)?;
        self.trail
            .records
            .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 0))));
        let stored = self.stored(item)?;
        self.trail.changes.push(ItemChange::Created(stored.clone()));
        self.modify_points(wear, &stored, proto, true);
        self.recompute(item.vnum);
        Ok(())
    }
}

/// The item types whose `UseItemEx` arm is `EquipItem` or `UnequipItem`
/// (`G/char_item.cpp:3040-3053`).
const USED_BY_WEARING: [i32; 8] = [
    ITEM_COSTUME,
    ITEM_WEAPON,
    ITEM_ARMOR,
    ITEM_ROD,
    ITEM_RING,
    ITEM_BELT,
    ITEM_TALISMAN,
    ITEM_PICK,
];

/// The book vnums `read_skill_book` arms (`G/char_item.cpp:4407-4790`), plus every
/// `ITEM_SKILLBOOK` (the socket/value arm, `:3282-3331`). A book-shaped item of another
/// vnum runs no read: the consume below would eat a book legacy reads as nothing.
const BOOK_VNUMS: [u32; 50] = [
    50060, 50061, 50600, 50301, 50302, 50303, 50304, 50305, 50306, 50311, 50312, 50313, 50314,
    50315, 50316, 50323, 50324, 50325, 50326, 50335, 50336, 50337, 50338, 50339, 50340, 50401,
    50402, 50403, 50404, 50405, 50406, 50407, 50408, 50409, 50410, 50411, 50412, 50413, 50414,
    50415, 50416, 50417, 50418, 50419, 50420, 50421, 50422, 50423, 50424, 50425,
];

/// Whether `CG_ITEM_USE` on an item of this prototype reads a skill book (230g).
#[must_use]
pub fn is_skill_book(proto: &ItemProto) -> bool {
    if proto.item_type == ITEM_SKILLBOOK {
        return true;
    }
    proto.item_type == ITEM_USE && proto.sub_type == USE_SPECIAL && BOOK_VNUMS.contains(&proto.vnum)
}

/// Whether `CG_ITEM_USE` on an item of this prototype forgets a skill (`:3262-3279`):
/// `ITEM_SKILLFORGET` (the 70037 of the owner's data).
#[must_use]
pub fn is_skill_forget(proto: &ItemProto) -> bool {
    proto.item_type == gamedata::item_kind::ITEM_SKILLFORGET
}

/// Whether the proto is a polymorph ball or book: `ITEM_POLYMORPH` of the ball five
/// or the 50322 book (`ItemProcess_Polymorph`, `G/char_item.cpp:9783-9814`, `:9816-9835`).
#[must_use]
pub fn is_polymorph_ball(proto: &ItemProto) -> bool {
    proto.item_type == ITEM_POLYMORPH && matches!(proto.vnum, 70_104..=70_107 | 71_093 | 50_322)
}

/// The ten seated `ITEM_USE` arms, in the order `use_special_kind` answers them: the
/// nine consuming arms, then the silent pair.
type SeatedArms = (bool, bool, bool, bool, bool, bool, bool, bool, bool, bool);

/// The use's classification: the potion, the skill book, the quest item, the
/// polymorph ball or book, and the ten seated arms.
type UseClassed = (bool, bool, bool, bool, bool, SeatedArms);

/// The unported-use refusal for the classified proto, or `None` when some arm
/// answers it: an `ITEM_USE` of no known sub type names the sub type, any other
/// unanswered type names the type.
fn use_ported(proto: &ItemProto, classed: UseClassed) -> Option<MoveRefused> {
    let (potion, book, forget, quest, ball, seated) = classed;
    let (clear, cloak, abled, euro, timed, shop_premium, barred, caped, nogged, silent) = seated;
    if !potion
        && !book
        && !clear
        && !cloak
        && !abled
        && !euro
        && !timed
        && !shop_premium
        && !barred
        && !caped
        && !nogged
        && !silent
        && proto.item_type == gamedata::item_kind::ITEM_USE
    {
        return Some(MoveRefused::NotPorted(Unported::UseSubType(proto.sub_type)));
    }
    if !potion
        && !book
        && !forget
        && !quest
        && !ball
        && !clear
        && !cloak
        && !abled
        && !euro
        && !timed
        && !shop_premium
        && !barred
        && !caped
        && !nogged
        && !silent
        && !USED_BY_WEARING.contains(&proto.item_type)
    {
        return Some(MoveRefused::NotPorted(Unported::Use(proto.item_type)));
    }
    None
}

/// The consumed kind of the seated `ITEM_USE` arms (or the skill book, or the
/// forget book), or `None` when the use is no seated arm: the cure, the cloak, the ability potion, the
/// 50801-50820 pair, the 507 arm, the gold bar, the cape, the pocket, else the plain
/// book's `Used` (the forget book's too).
fn seated_use_kind(
    book: bool,
    forget: bool,
    seated: SeatedArms,
) -> Option<super::item_move::MoveKind> {
    use super::item_move::MoveKind;
    let (clear, cloak, abled, euro, timed, shop_premium, barred, caped, nogged, _silent) = seated;
    if !(book
        || forget
        || clear
        || cloak
        || abled
        || euro
        || timed
        || shop_premium
        || barred
        || caped
        || nogged)
    {
        return None;
    }
    Some(if clear {
        MoveKind::Cleared
    } else if cloak {
        MoveKind::Cloaked
    } else if abled {
        MoveKind::Abled
    } else if euro {
        MoveKind::EuroFree
    } else if timed {
        MoveKind::Timed
    } else if shop_premium {
        MoveKind::ShopPremium
    } else if barred {
        MoveKind::Barred
    } else if caped {
        MoveKind::Caped
    } else if nogged {
        MoveKind::Nogged
    } else {
        MoveKind::Used
    })
}

/// The game-state-seated `ITEM_USE` arms of the proto: the `USE_CLEAR` cure, the
/// `USE_INVISIBILITY` cloak, the `USE_ABILITY_UP` potion, the 50801-50820
/// "additional effect" pair (`G/char_item.cpp:3371-3521`), the main-switch
/// `USE_AFFECT` (`:6833-6864`), the 507 premium-shop arm (`:6835-6843`), and the
/// gold bars (`LINGOURI_SURSA`, `:3328-3370`), the cape of courage
/// (`case USE_SPECIAL`'s 70038/70057/76007 arm, `:3879-3888`), and the Nog
/// pocket (50216, `:3787-3811`), then the silent pair (`USE_BUCKET` and
/// `USE_CHANGE_ATTRIBUTE_PLUS`, with no arm), in that order. The branch's
/// `USE_POTION_NODELAY` runs the ordinary nodelay arm, and its `USE_POTION`
/// answers nothing.
fn use_special_kind(proto: &ItemProto) -> SeatedArms {
    use gamedata::item_kind::{
        ITEM_USE, USE_ABILITY_UP, USE_AFFECT, USE_BUCKET, USE_CHANGE_ATTRIBUTE_PLUS, USE_CLEAR,
        USE_INVISIBILITY,
    };
    if proto.item_type != ITEM_USE {
        return (
            false, false, false, false, false, false, false, false, false, false,
        );
    }
    if (GOLD_BAR_LOW..=GOLD_BAR_HIGH).contains(&proto.vnum) {
        return (
            false, false, false, false, false, false, true, false, false, false,
        );
    }
    if CAPE_OF_COURAGE.contains(&proto.vnum) {
        return (
            false, false, false, false, false, false, false, true, false, false,
        );
    }
    if proto.vnum == NOG_POCKET {
        return (
            false, false, false, false, false, false, false, false, true, false,
        );
    }
    if (50_801..=50_820).contains(&proto.vnum) {
        let euro = proto.sub_type == USE_ABILITY_UP || proto.sub_type == USE_AFFECT;
        return (
            false, false, false, euro, false, false, false, false, false, false,
        );
    }
    let shop_premium =
        proto.sub_type == USE_AFFECT && proto.values[0] == AFFECT_PREMIUM_PRIVATE_SHOP_VALUE;
    let timed = proto.sub_type == USE_AFFECT && !shop_premium;
    let silent = proto.sub_type == USE_BUCKET || proto.sub_type == USE_CHANGE_ATTRIBUTE_PLUS;
    (
        proto.sub_type == USE_CLEAR,
        proto.sub_type == USE_INVISIBILITY,
        proto.sub_type == USE_ABILITY_UP,
        false,
        timed,
        shop_premium,
        false,
        false,
        false,
        silent,
    )
}

/// The Nog pocket's vnum (`ITEM_NOG_POCKET`, `G/unique_item.h:98`).
const NOG_POCKET: u32 = 50_216;

/// The cape of courage's three vnums: 70038, 70057 and the reward box's 76007
/// (`G/char_item.cpp:3879-3882`, `G/unique_item.h:23,107`).
const CAPE_OF_COURAGE: [u32; 3] = [70_038, 70_057, 76_007];

/// The gold bars' vnums (`case 80003` .. `case 80008`, `G/char_item.cpp:3335-3340`).
const GOLD_BAR_LOW: u32 = 80_003;
/// The last gold bar's vnum.
const GOLD_BAR_HIGH: u32 = 80_008;

/// `AFFECT_PREMIUM_PRIVATE_SHOP` (`G/affect.h:66`): 507, the premium-shop arm's
/// value(0).
const AFFECT_PREMIUM_PRIVATE_SHOP_VALUE: i32 = 507;

/// The `ITEM_QUEST` the `RENEWAL_PICKUP_AFFECT` arm of `UseItemEx` takes
/// (`G/char_item.cpp:2852-2872`). It leaves here as any quest item does, and the caller runs
/// the arm in place of the item's quest.
pub const PICKUP_AFFECT_ITEM: u32 = 70_002;

/// Use one of the stack and answer `kind`: the shared consume of the skill books,
/// the `USE_CLEAR` cure, the `USE_INVISIBILITY` cloak and the `USE_ABILITY_UP`
/// potion, whose effects run on the game state.
fn consume_one_use(
    items: &mut CharacterItems,
    item: &Item,
    proto: &ItemProto,
    kind: super::item_move::MoveKind,
    stamped: &mut super::item_move::MoveDone,
) -> Result<super::item_move::MoveDone, MoveRefused> {
    let mut trail = Trail::default();
    super::potion::use_one(
        items,
        item,
        super::quickslot::chains_when_used_up(proto),
        &mut trail,
    )?;
    trail.records.splice(0..0, stamped.records.drain(..));
    trail.changes.splice(0..0, stamped.changes.drain(..));
    Ok(super::item_move::done(kind, trail))
}

/// Run one `CG_ITEM_USE` on an item that is worn by using it: `CHARACTER::UseItem` and the
/// equip arm of `UseItemEx` (`G/char_item.cpp:7168-7400`, `:2718-3053`). An item that is
/// not worn goes on (`EquipItem`), and a worn one comes off (`UnequipItem`).
///
/// The checks run in legacy's order: an item offered in a trade (`IsExchanging`, refused
/// silently), the job (`CanUsedBy`), the sex, a script of the character that waits for its
/// client (`@fixme150`, `G/char_item.cpp:7350`), the level and conqueror level limits, and the
/// belt cell the item is used from. A first-use item's clock stamps next, before the belt
/// arms and the type switch (`G/char_item.cpp:2804-2820`); a later refusal then lands as
/// `Declined`, with the stamps leading. Then `EquipItem` or `UnequipItem` run their own
/// checks as a move's do. An `ITEM_QUEST` stops after the checks with `MoveKind::QuestUsed`
/// and changes nothing; the caller runs its quest. Legacy's checks for states this build
/// does not have (a shop, a cube, a stun, the secured account) never refuse here; the
/// stack flood gates above, in the game state.
/// # Errors
///
/// [`MoveRefused::NotPorted`] for an item of a type whose use arm is not ported
/// ([`Unported::Use`], among them `ITEM_UNIQUE` and the dragon soul stones), for an item in a
/// window other than the inventory and the equipment, and without `gear`. Otherwise any
/// refusal of the checks, which changes nothing, as [`move_item`](super::move_item)'s do.
pub fn use_item(
    items: &mut CharacterItems,
    at: ItemPos,
    rules: &super::item_move::MoveRules,
    gear: Option<&mut Gear<'_>>,
) -> Result<super::item_move::MoveDone, MoveRefused> {
    use super::item_move::{declined, done, MoveKind};

    let item = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
    let proto = gear
        .protos
        .get(item.vnum)
        .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    let classed = (
        super::potion::is_potion(proto),
        is_skill_book(proto),
        is_skill_forget(proto),
        proto.item_type == gamedata::item_kind::ITEM_QUEST,
        is_polymorph_ball(proto),
        use_special_kind(proto),
    );
    if let Some(refused) = use_ported(proto, classed) {
        return Err(refused);
    }
    let (potion, book, forget, quest, ball, seated) = classed;
    use_checks(&item, proto, rules, gear.points)?;
    // `UseItemEx`'s first-use starts (`G/char_item.cpp:2804-2820`): the clock stamps after
    // the checks, before the belt arms and the type switch. Without a clock base the use
    // is refused, as before; with one the stamps lead the answer, in legacy's order.
    let mut stamped = super::item_move::MoveDone {
        kind: MoveKind::Used,
        records: Vec::new(),
        changes: Vec::new(),
    };
    if proto.real_time_first_use.is_some() {
        let Some(now) = gear.first_use_now else {
            return Err(MoveRefused::NotPorted(Unported::Worn(WornSystem::Timer)));
        };
        let value = proto
            .limits
            .iter()
            .find(|limit| limit.kind == gamedata::item_kind::LIMIT_REAL_TIME_START_FIRST_USE)
            .map_or(0, |limit| limit.value);
        stamped = stamp_first_use_clock(items, &item, value, now);
    }
    // The stamp landed in `items`: the arms below read the stamped sockets, as the
    // equip checks do when they admit a started first-use clock.
    let item = items.item(item.id).cloned().unwrap_or(item);
    if quest {
        // `case ITEM_QUEST` changes nothing here: the caller runs the item's quest, or the
        // pick-up arm `RENEWAL_PICKUP_AFFECT` gives 70002 (`G/char_item.cpp:2852-2872`).
        return Ok(done(MoveKind::QuestUsed, Trail::default()));
    }
    if potion {
        let mut used = super::potion::use_potion(items, &item, proto, &mut *gear.points)?;
        let mut records = core::mem::take(&mut stamped.records);
        records.append(&mut used.records);
        used.records = records;
        let mut changes = core::mem::take(&mut stamped.changes);
        changes.append(&mut used.changes);
        used.changes = changes;
        return Ok(used);
    }
    if ball && proto.vnum != 50_322 {
        // `case ITEM_POLYMORPH`'s ball arm (`G/char_item.cpp:9783-9814`) uses one of
        // the stack; the gates refused before the step, and the game state lands the
        // morph and the bonus after it, so this arm answers the consume and the kind
        // names the land. The book (50322) stops here: the game state runs its chance
        // and keeps it, as its count never falls.
        return consume_one_use(items, &item, proto, MoveKind::Morphed, &mut stamped);
    }
    if ball {
        return Ok(done(MoveKind::Booked, Trail::default()));
    }
    if let Some(kind) = seated_use_kind(book, forget, seated) {
        // `case USE_CLEAR` (`G/char_item.cpp:5539-5551`) strips the bad affects and
        // uses one of the stack; the strip runs on the game state (it holds the
        // affects), so this arm answers the consume and the kind names the strip.
        // `case USE_INVISIBILITY` (`:5553-5577`) cloaks for 300 s the same way, and
        // `case USE_ABILITY_UP` (`:5780-5859`) lands the apply's affect the same way.
        // The 50801-50820 pair (`:3371-3521`) lands 516 the same way, `case
        // USE_AFFECT` (`:6833-6864`) lands value(0) the same way, and the 507 arm
        // (`:6835-6843`) runs the setter the same way. The gold bar pays its gold
        // the same way, after the game state's refusals, and the cape of courage
        // gathers the monsters the same way. The Nog pocket lands its three rows
        // the same way, after the held refusal.
        return consume_one_use(items, &item, proto, kind, &mut stamped);
    }
    // `USE_BUCKET` and `USE_CHANGE_ATTRIBUTE_PLUS` have no arm in the subtype switch
    // (`G/char_item.cpp:3572-7047` has no `default`), so the `case ITEM_USE` falls through
    // and `UseItemEx` returns true at `:7162`: legacy keeps the item and chats nothing.
    // The use's checks already ran, and the game state counts the pass hook past the
    // step, as it does for every landed use.
    let (_, _, _, _, _, _, _, _, _, silent) = seated;
    if silent {
        let mut trail = Trail::default();
        trail.records.append(&mut stamped.records);
        trail.changes.append(&mut stamped.changes);
        return Ok(done(MoveKind::Used, trail));
    }
    let mut equipper = Equipper::new(items, gear, rules.usable_cells);
    let outcome = if super::inventory::is_equip_position(item.pos) {
        equipper.unequip_item(&item).map(|()| MoveKind::Unequipped)
    } else {
        equipper.equip_item(&item).map(|swapped| {
            if swapped {
                MoveKind::Swapped
            } else {
                MoveKind::Equipped
            }
        })
    };
    let mut trail = equipper.into_trail();
    trail.records.splice(0..0, stamped.records.drain(..));
    trail.changes.splice(0..0, stamped.changes.drain(..));
    if outcome.is_err() && trail.records.is_empty() && trail.changes.is_empty() {
        let Err(refused) = outcome else {
            unreachable!()
        };
        return Err(refused);
    }
    match outcome {
        Ok(kind) => Ok(done(kind, trail)),
        Err(refused) => declined(refused, trail),
    }
}

/// Spend one giftbox of the stack at `at`: the `SetCount(item->GetCount() - 1)`
/// of a drawn `ITEM_GIFTBOX` use (`G/char_item.cpp:3215`). The game state runs the
/// draw first and calls this only once the draw's arms have all landed, as legacy
/// consumes only when `GiveItemFromSpecialItemGroup` returns true.
///
/// # Errors
///
/// As [`use_item`]: a missing or foreign source, no gear, an unknown vnum, or a
/// refused use.
pub fn consume_giftbox(
    items: &mut CharacterItems,
    at: ItemPos,
    rules: &super::item_move::MoveRules,
    gear: Option<&mut Gear<'_>>,
) -> Result<super::item_move::MoveDone, MoveRefused> {
    let item = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
    let proto = gear
        .protos
        .get(item.vnum)
        .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    use_checks(&item, proto, rules, gear.points)?;
    let mut stamped = super::item_move::MoveDone {
        kind: super::item_move::MoveKind::Used,
        records: Vec::new(),
        changes: Vec::new(),
    };
    consume_one_use(
        items,
        &item,
        proto,
        super::item_move::MoveKind::Used,
        &mut stamped,
    )
}

/// Check the key at `at` before its draw: the source and the use checks, with
/// nothing spent. The game state calls this before the box's draw, whose arms
/// have side effects, and spends both only after they land.
///
/// # Errors
///
/// As [`use_item`]: a missing or foreign source, no gear, an unknown vnum, or a
/// refused use.
pub fn check_key_use(
    items: &mut CharacterItems,
    at: ItemPos,
    rules: &super::item_move::MoveRules,
    gear: Option<&mut Gear<'_>>,
) -> Result<super::item_move::MoveDone, MoveRefused> {
    let item = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
    let proto = gear
        .protos
        .get(item.vnum)
        .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    use_checks(&item, proto, rules, gear.points)?;
    Ok(super::item_move::MoveDone {
        kind: super::item_move::MoveKind::Used,
        records: Vec::new(),
        changes: Vec::new(),
    })
}

/// Spend the key at `at` and the box at `dest`: the two `RemoveItem` calls of a
/// drawn `ITEM_TREASURE_KEY` use (`G/char_item.cpp:3125-3126`), the key first.
///
/// # Errors
///
/// As [`use_item`]: a missing or foreign source, no gear, an unknown vnum, or a
/// refused use.
pub fn consume_key_and_box(
    items: &mut CharacterItems,
    at: ItemPos,
    dest: ItemPos,
    rules: &super::item_move::MoveRules,
    gear: Option<&mut Gear<'_>>,
) -> Result<super::item_move::MoveDone, MoveRefused> {
    let key = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
    let proto = gear
        .protos
        .get(key.vnum)
        .ok_or(MoveRefused::UnknownVnum(key.vnum))?;
    use_checks(&key, proto, rules, gear.points)?;
    let mut stamped = super::item_move::MoveDone {
        kind: super::item_move::MoveKind::Used,
        records: Vec::new(),
        changes: Vec::new(),
    };
    let mut spent = consume_one_use(
        items,
        &key,
        proto,
        super::item_move::MoveKind::Used,
        &mut stamped,
    )?;
    let box_item = use_source(items, dest)?;
    let box_proto = gear
        .protos
        .get(box_item.vnum)
        .ok_or(MoveRefused::UnknownVnum(box_item.vnum))?;
    let mut box_spent = consume_one_use(
        items,
        &box_item,
        box_proto,
        super::item_move::MoveKind::Used,
        &mut stamped,
    )?;
    spent.records.append(&mut box_spent.records);
    spent.changes.append(&mut box_spent.changes);
    Ok(spent)
}
const EXPIRY_SOCKET: usize = 0;

/// The use counter (`UseItemEx`'s socket 1, `G/char_item.cpp:2809-2819`).
const USE_SOCKET: usize = 1;

/// A missing duration defaults to a week (`G/char_item.cpp:2816`).
const DEFAULT_FIRST_USE_SECS: i64 = 7 * 24 * 60 * 60;

/// The first-use stamp (`G/char_item.cpp:2809-2819`) of `item` at `now`: socket 0 when
/// socket 1 is 0, then socket 1 when the item is not worn. Each set sends and saves.
fn stamp_first_use_clock(
    items: &mut CharacterItems,
    item: &crate::item::Item,
    limit_value: i32,
    now: u32,
) -> super::item_move::MoveDone {
    use super::item_move::MoveKind;

    let mut done = super::item_move::MoveDone {
        kind: MoveKind::Used,
        records: Vec::new(),
        changes: Vec::new(),
    };
    if item.sockets[USE_SOCKET] == 0 {
        let span = i64::from(item.sockets[EXPIRY_SOCKET]).max(0);
        let span = if span == 0 {
            i64::from(limit_value).max(0)
        } else {
            span
        };
        let span = if span == 0 {
            DEFAULT_FIRST_USE_SECS
        } else {
            span
        };
        #[allow(clippy::cast_possible_truncation)]
        let at = i64::from(now).saturating_add(span) as i32;
        let step = super::specify_item::set_item_socket(items, item.id, (EXPIRY_SOCKET, at));
        done.records.extend(step.records);
        done.changes.extend(step.changes);
    }
    if wear_of(item.pos).is_none() {
        let uses = item.sockets[USE_SOCKET].saturating_add(1);
        let step = super::specify_item::set_item_socket(items, item.id, (USE_SOCKET, uses));
        done.records.extend(step.records);
        done.changes.extend(step.changes);
    }
    done
}

/// `UseItem`'s source checks (`G/char_item.cpp:7177-7190`): a valid cell of a window this
/// build lays flat, holding an item that is not offered in a trade.
///
/// # Errors
///
/// [`MoveRefused::InvalidSource`], [`MoveRefused::NotPorted`] for another window,
/// [`MoveRefused::Empty`] and [`MoveRefused::Exchanging`].
pub fn use_source(items: &CharacterItems, at: ItemPos) -> Result<Item, MoveRefused> {
    if !super::inventory::is_valid_item_position(at) {
        return Err(MoveRefused::InvalidSource);
    }
    if !super::item_move::is_flat_window(at.window_type) {
        return Err(MoveRefused::NotPorted(Unported::SourceWindow(
            at.window_type,
        )));
    }
    let item = items.item_at(at).cloned().ok_or(MoveRefused::Empty)?;
    if items.is_exchanging(item.id) {
        return Err(MoveRefused::Exchanging);
    }
    Ok(item)
}

/// `UseItem`'s and `UseItemEx`'s checks of the item used, in legacy's order, after the
/// source cell, the trade and the proto: the job (`CanUsedBy`), the sex, a waiting script
/// (`@fixme150`), the level and conqueror level limits, and the belt cell
/// (`G/char_item.cpp:7168-7400`, `:2718-2800`). A first-use item passes here; the caller
/// stamps its clock after these checks, as `UseItemEx` does (`:2804-2820`).
///
/// # Errors
///
/// The first check that refuses; none changes anything.
pub fn use_checks(
    item: &Item,
    proto: &ItemProto,
    rules: &super::item_move::MoveRules,
    points: &Points,
) -> Result<(), MoveRefused> {
    let race = points.race();
    if proto.anti_flags & job_anti_flag(race) != 0 {
        return Err(MoveRefused::NotUsableByJob);
    }
    if proto.anti_flags & sex_anti_flag(race) != 0 {
        return Err(MoveRefused::WrongSex);
    }
    if rules.questing {
        return Err(MoveRefused::UsedWhileQuesting);
    }
    for limit in &proto.limits {
        let (have, refused) = match limit.kind {
            LIMIT_LEVEL => (i32::from(points.level()), MoveRefused::UseLevelTooLow),
            LIMIT_CHAMPION => (
                i32::from(points.conqueror_level()),
                MoveRefused::ChampionTooLow,
            ),
            _ => continue,
        };
        if have < limit.value {
            return Err(refused);
        }
    }
    if super::inventory::is_belt_inventory_position(item.pos) {
        let grade = rules.belt_grade.ok_or(MoveRefused::NoBeltWorn)?;
        if !super::items::belt_cell_is_available(item.pos.cell - BELT_INVENTORY_SLOT_START, grade) {
            return Err(MoveRefused::BeltCellLocked);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use gamedata::item_kind::{ARMOR_BODY, ITEM_ANTIFLAG_MALE, LIMIT_LEVEL};
    use gamedata::item_proto::ItemValue;

    use super::*;
    use crate::character::item_move::{move_item, MoveDone, MoveKind, MoveRequest, MoveRules};
    use crate::character::item_move::{MoveFacts, MoveRecord, MOVE_SWITCHBOT_ITEM};
    use crate::character::points::PointsRow;

    /// No groups: the sig tail appends nothing.
    static GROUPS: std::sync::LazyLock<gamedata::special_item_group::SpecialItemGroups> =
        std::sync::LazyLock::new(gamedata::special_item_group::SpecialItemGroups::default);

    const INV: u8 = EWindows::Inventory as u8;
    const WEAPON: u16 = EWearPositions::Weapon as u16;
    const BODY: u16 = EWearPositions::Body as u16;
    const SASH: u16 = EWearPositions::CostumeSash as u16;
    const COSTUME_BODY_CELL: u16 = EWearPositions::CostumeBody as u16;
    const MOUNT_CELL: u16 = EWearPositions::CostumeMount as u16;

    const SWORD: u32 = 11;
    const ARMOUR: u32 = 11_210;
    const HEAVY_ARMOUR: u32 = 11_220;
    const HIGH_SWORD: u32 = 21;
    const SASH_GRADE_4: u32 = 85_004;
    const SASH_GRADE_2: u32 = 85_002;
    const COSTUME: u32 = 41_001;
    const WEDDING: u32 = 11_901;
    /// A mount costume without the first-use timer the owner's rows carry.
    const MOUNT_COSTUME: u32 = 52_001;
    /// A mount costume with the first-use timer, like the owner's rows.
    const TIMED_MOUNT: u32 = 52_002;
    const MALE_ONLY: u32 = 11_230;
    const UNIQUE: u32 = 71_001;
    /// A real-time unique: its expiry is another event, so it still refuses.
    const REAL_TIME_UNIQUE: u32 = 71_002;
    /// The hide-alignment cloak (`G/unique_item.h:28`): the one unique that wears.
    const HIDE_ALIGNMENT_CLOAK: u32 = 70_048;
    /// A marriage unique (`G/marriage.cpp:24-29`): worn, it needs a married wearer.
    const MARRIAGE_UNIQUE: u32 = 71_069;
    const RING: u32 = 70_001;
    const LEVEL_TEN_SWORD: u32 = 31;
    const UNIQUE_FLAGGED: u32 = 11_240;
    const SPECIAL: u32 = 71_049;
    /// A book-shaped `ITEM_USE`/`USE_SPECIAL` in the read set: the summon book.
    const BOOK: u32 = 50_061;
    /// A `USE_CLEAR` cure: the owner's 27124.
    const CURE: u32 = 27_124;
    /// A `USE_INVISIBILITY` cloak: the owner's 27877.
    const CLOAK: u32 = 27_877;
    /// A `USE_ABILITY_UP` potion: the owner's 27053 (attack speed).
    const ABLED: u32 = 27_053;
    /// A 508xx `USE_ABILITY_UP` potion: the owner's 50820 (attack speed).
    const EURO_ABLED: u32 = 50_820;
    /// A 508xx `USE_AFFECT` potion: the owner's 50813 (critical).
    const EURO_AFFECT: u32 = 50_813;
    /// A main-switch `USE_AFFECT` item: the owner's 39010 (mall attack speed).
    const TIMED: u32 = 39_010;
    /// A 507 premium-shop item: the owner's 72355.
    const SHOP_PREMIUM: u32 = 72_355;
    /// A gold bar: the owner's 80003 (500,000 yang).
    const GOLD_BAR: u32 = 80_003;
    /// A cape of courage: the owner's 70038.
    const CAPE: u32 = 70_038;
    /// A Nog pocket: the owner's 50216.
    const NOG: u32 = 50_216;
    const POTION: u32 = 27_101;
    const HIGH_POTION: u32 = 27_102;
    const SURA_REFUSED: u32 = 12;
    const CHAMPION_SWORD: u32 = 41;
    const TIMED_SWORD: u32 = 51;
    const MALE_ONLY_HIGH: u32 = 11_250;
    const COSTUME_BLADE: u32 = 40_001;
    /// An `ITEM_QUEST`, the boost and ward reset book, and one with a level limit.
    const QUEST_BOOK: u32 = 71_000;
    const HIGH_QUEST_BOOK: u32 = 71_001_000;

    const RULES: MoveRules = MoveRules {
        count_limit: 200,
        usable_cells: 90,
        belt_grade: None,
        questing: false,
        secured: false,
    };

    /// Always draws the same number.
    struct Fixed(u32);

    impl Dice for Fixed {
        fn random31(&mut self) -> u32 {
            self.0
        }
    }

    fn proto(vnum: u32, item_type: i32, sub_type: i32, wear_flags: u32) -> ItemProto {
        let mut proto = ItemProto::for_category_rule(vnum, item_type, sub_type);
        proto.wear_flags = wear_flags;
        proto
    }

    /// The `ITEM_USE` consumables of the test protos: the special, the cure, the
    /// cloak, the ability potion, and the two nodelay potions.
    fn use_rows() -> Vec<ItemProto> {
        // `USE_POTION_NODELAY`, 11: 100 hit points at once.
        let mut potion = proto(POTION, 3, 11, 0);
        potion.values[0] = 100;
        let mut high_potion = potion.clone();
        high_potion.vnum = HIGH_POTION;
        high_potion.limits[0] = ItemValue {
            kind: LIMIT_LEVEL,
            value: 30,
        };
        vec![
            // `ITEM_USE`, 3, `USE_SPECIAL`, 10.
            proto(SPECIAL, 3, 10, 0),
            proto(BOOK, 3, 10, 0),
            // The gold bar: `USE_SPECIAL`, 10.
            proto(GOLD_BAR, 3, 10, 0),
            // The cape of courage: `USE_SPECIAL`, 10.
            proto(CAPE, 3, 10, 0),
            // The Nog pocket: `USE_SPECIAL`, 10.
            proto(NOG, 3, 10, 0),
            // `USE_CLEAR`, 12.
            proto(CURE, 3, 12, 0),
            // `USE_INVISIBILITY`, 13.
            proto(CLOAK, 3, 13, 0),
            // `USE_ABILITY_UP`, 7.
            proto(ABLED, 3, 7, 0),
            // The 50801-50820 pair: `USE_ABILITY_UP` 7 and `USE_AFFECT` 8.
            proto(EURO_ABLED, 3, 7, 0),
            proto(EURO_AFFECT, 3, 8, 0),
            // The main-switch `USE_AFFECT`, 8.
            proto(TIMED, 3, 8, 0),
            // The 507 premium-shop arm, 8 with value(0) 507: the owner's 72355
            // runs a week, 604800 s.
            {
                let mut premium = proto(SHOP_PREMIUM, 3, 8, 0);
                premium.values[0] = 507;
                premium.values[3] = 604_800;
                premium
            },
            potion,
            high_potion,
        ]
    }

    fn protos() -> ItemProtos {
        let mut armour = proto(ARMOUR, ITEM_ARMOR, ARMOR_BODY, WEARABLE_BODY);
        armour.values[1] = 40;
        // `APPLY_MOV_SPEED`, 8.
        armour.applies[0] = ItemValue { kind: 8, value: -2 };
        let mut heavy = proto(HEAVY_ARMOUR, ITEM_ARMOR, ARMOR_BODY, WEARABLE_BODY);
        heavy.values[1] = 70;
        let mut high = proto(HIGH_SWORD, ITEM_WEAPON, 0, WEARABLE_WEAPON);
        high.limits[0] = ItemValue {
            kind: LIMIT_LEVEL,
            value: 30,
        };
        let mut level_ten = proto(LEVEL_TEN_SWORD, ITEM_WEAPON, 0, WEARABLE_WEAPON);
        level_ten.limits[0] = ItemValue {
            kind: LIMIT_LEVEL,
            value: 10,
        };
        let mut grade_4 = proto(SASH_GRADE_4, ITEM_COSTUME, COSTUME_SASH, 0);
        grade_4.values[0] = 4;
        let mut grade_2 = proto(SASH_GRADE_2, ITEM_COSTUME, COSTUME_SASH, 0);
        grade_2.values[0] = 2;
        let mut sura_refused = proto(SURA_REFUSED, ITEM_WEAPON, 0, WEARABLE_WEAPON);
        sura_refused.anti_flags = gamedata::item_kind::ITEM_ANTIFLAG_WARRIOR;
        let mut champion = proto(CHAMPION_SWORD, ITEM_WEAPON, 0, WEARABLE_WEAPON);
        champion.limits[1] = ItemValue {
            kind: gamedata::item_kind::LIMIT_CHAMPION,
            value: 1,
        };
        let mut timed = proto(TIMED_SWORD, ITEM_WEAPON, 0, WEARABLE_WEAPON);
        timed.limits[0] = ItemValue {
            kind: gamedata::item_kind::LIMIT_REAL_TIME_START_FIRST_USE,
            value: 0,
        };
        timed.real_time_first_use = Some(0);
        let mut timed_mount = proto(TIMED_MOUNT, ITEM_COSTUME, COSTUME_MOUNT, 0);
        timed_mount.limits[0] = ItemValue {
            kind: gamedata::item_kind::LIMIT_REAL_TIME_START_FIRST_USE,
            value: 0,
        };
        timed_mount.real_time_first_use = Some(0);
        let mut male_only = proto(MALE_ONLY, ITEM_ARMOR, ARMOR_BODY, WEARABLE_BODY);
        male_only.anti_flags = ITEM_ANTIFLAG_MALE;
        let mut male_only_high = proto(MALE_ONLY_HIGH, ITEM_ARMOR, ARMOR_BODY, WEARABLE_BODY);
        male_only_high.anti_flags = ITEM_ANTIFLAG_MALE;
        male_only_high.limits[0] = ItemValue {
            kind: LIMIT_LEVEL,
            value: 30,
        };
        let mut rows = use_rows();
        // A costume weapon for a weapon sub type other than the sword's 0.
        let mut costume_blade = proto(COSTUME_BLADE, ITEM_COSTUME, COSTUME_WEAPON, 0);
        costume_blade.values[3] = 1;
        let mut high_quest = proto(HIGH_QUEST_BOOK, gamedata::item_kind::ITEM_QUEST, 0, 0);
        high_quest.limits[0] = ItemValue {
            kind: LIMIT_LEVEL,
            value: 30,
        };
        let mut real_time_unique = proto(REAL_TIME_UNIQUE, ITEM_UNIQUE, 0, WEARABLE_UNIQUE);
        real_time_unique.limits[0] = ItemValue {
            kind: gamedata::item_kind::LIMIT_REAL_TIME,
            value: 3600,
        };
        let mut all = vec![
            proto(SWORD, ITEM_WEAPON, 0, WEARABLE_WEAPON),
            proto(QUEST_BOOK, gamedata::item_kind::ITEM_QUEST, 0, 0),
            proto(PICKUP_AFFECT_ITEM, gamedata::item_kind::ITEM_QUEST, 0, 0),
            high_quest,
            armour,
            heavy,
            high,
            grade_4,
            grade_2,
            proto(COSTUME, ITEM_COSTUME, COSTUME_BODY, 0),
            proto(MOUNT_COSTUME, ITEM_COSTUME, COSTUME_MOUNT, 0),
            timed_mount,
            proto(WEDDING, ITEM_ARMOR, ARMOR_BODY, WEARABLE_BODY),
            male_only,
            proto(UNIQUE, ITEM_UNIQUE, 0, WEARABLE_UNIQUE),
            real_time_unique,
            proto(HIDE_ALIGNMENT_CLOAK, ITEM_UNIQUE, 0, WEARABLE_UNIQUE),
            proto(MARRIAGE_UNIQUE, ITEM_UNIQUE, 0, WEARABLE_UNIQUE),
            proto(RING, ITEM_RING, 0, 0),
            level_ten,
            proto(UNIQUE_FLAGGED, ITEM_ARMOR, ARMOR_BODY, WEARABLE_UNIQUE),
            sura_refused,
            champion,
            timed,
            male_only_high,
            costume_blade,
        ];
        all.append(&mut rows);
        ItemProtos::from_rows(all)
    }

    fn warrior() -> Points {
        Points::load(&PointsRow {
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
            premium: (0, 0),
            shop_unlocked: 0,
        })
    }

    const fn inv(cell: u16) -> ItemPos {
        ItemPos {
            window_type: INV,
            cell,
        }
    }

    fn holding(placed: &[(ItemPos, Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (pos, item) in placed {
            items.set(*pos, item).expect("the fixture places");
        }
        items
    }

    /// A character, its storage and the move it asks for.
    struct Case {
        items: CharacterItems,
        points: Points,
        recently_fought: bool,
    }

    impl Case {
        fn new(placed: &[(ItemPos, Item)]) -> Self {
            Self {
                items: holding(placed),
                points: warrior(),
                recently_fought: false,
            }
        }

        fn wearing(worn: &[(u16, Item)]) -> Self {
            let mut case = Self::new(&[]);
            let protos = protos();
            for (wear, item) in worn {
                case.items
                    .set(wear_pos(*wear), item)
                    .expect("the wear cell is free");
            }
            let _ = case
                .points
                .compute_points_with(&Equipment::of(&case.items, &protos));
            case
        }

        fn run(&mut self, from: ItemPos, to: ItemPos) -> Result<MoveDone, MoveRefused> {
            let protos = protos();
            let mut dice = Fixed(5);
            let mut gear = Gear {
                points: &mut self.points,
                protos: &protos,
                groups: &GROUPS,
                dice: &mut dice,
                recently_fought: self.recently_fought,
                first_use_now: Some(1_700_000_000),
            };
            let request = MoveRequest { from, to, count: 0 };
            let facts = MoveFacts {
                categories: Vec::new(),
                flags: MOVE_SWITCHBOT_ITEM,
            };
            move_item(
                &mut self.items,
                None,
                request,
                &RULES,
                Some(&mut gear),
                |_| Some(facts),
            )
        }

        /// `CG_ITEM_USE` on the item at `at`.
        fn use_at(&mut self, at: ItemPos) -> Result<MoveDone, MoveRefused> {
            self.use_with(at, &RULES)
        }

        fn use_with(&mut self, at: ItemPos, rules: &MoveRules) -> Result<MoveDone, MoveRefused> {
            let protos = protos();
            let mut dice = Fixed(5);
            let mut gear = Gear {
                points: &mut self.points,
                protos: &protos,
                groups: &GROUPS,
                dice: &mut dice,
                recently_fought: self.recently_fought,
                first_use_now: Some(1_700_000_000),
            };
            use_item(&mut self.items, at, rules, Some(&mut gear))
        }

        /// A use that must be refused, and change nothing.
        fn use_refused(&mut self, at: ItemPos) -> MoveRefused {
            let items = self.items.clone();
            let points = self.points.clone();
            let reason = self.use_at(at).expect_err("the use is refused");
            assert_eq!(self.items, items, "a refused use changed the storage");
            assert_eq!(self.points, points, "a refused use changed the points");
            reason
        }

        /// A move that must be refused, and change nothing.
        fn refused(&mut self, from: ItemPos, to: ItemPos) -> MoveRefused {
            let items = self.items.clone();
            let points = self.points.clone();
            let reason = self.run(from, to).expect_err("the move is refused");
            assert_eq!(self.items, items, "a refused move changed the storage");
            assert_eq!(self.points, points, "a refused move changed the points");
            reason
        }

        fn at(&self, pos: ItemPos) -> Option<u32> {
            self.items.item_at(pos).map(|item| item.id)
        }
    }

    fn item_records(done: &MoveDone) -> Vec<ItemRecord> {
        done.records
            .iter()
            .filter_map(|record| match record {
                MoveRecord::Item(record) => Some(*record),
                _ => None,
            })
            .collect()
    }

    fn looks(done: &MoveDone) -> Vec<CharacterLook> {
        done.records
            .iter()
            .filter_map(|record| match record {
                MoveRecord::Look(look) => Some(*look),
                _ => None,
            })
            .collect()
    }

    fn effects(done: &MoveDone) -> Vec<u8> {
        done.records
            .iter()
            .filter_map(|record| match record {
                MoveRecord::Effect(effect) => Some(*effect),
                _ => None,
            })
            .collect()
    }

    fn defence(points: &Points) -> i32 {
        points.get_point(point::POINT_DEF_GRADE)
    }

    /// A book-shaped use consumes one: the read gate lives above the world, so a book
    /// that reaches the world's step is eaten, with the clear and the destroy the potion
    /// path writes.
    #[test]
    fn a_book_shaped_use_consumes_one() {
        let book = Item::new(7, BOOK);
        let mut case = Case::new(&[(inv(3), book)]);
        let done = case.use_at(inv(3)).expect("a book is consumed");
        assert_eq!(done.kind, MoveKind::Used);
        assert_eq!(
            item_records(&done),
            vec![ItemRecord::Set(gc_item_clear(inv(3)))]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert_eq!(case.at(inv(3)), None);
    }
    /// A `USE_CLEAR` cure consumes one and answers `Cleared`: the game state strips
    /// the bad affects, because the affects live there, not in the storage.
    #[test]
    fn a_cure_use_consumes_one_and_names_the_strip() {
        let cure = Item::new(7, CURE);
        let mut case = Case::new(&[(inv(3), cure)]);
        let done = case.use_at(inv(3)).expect("a cure is consumed");
        assert_eq!(done.kind, MoveKind::Cleared);
        assert_eq!(
            item_records(&done),
            vec![ItemRecord::Set(gc_item_clear(inv(3)))]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert_eq!(case.at(inv(3)), None);
    }
    /// A `USE_INVISIBILITY` cloak consumes one and answers `Cloaked`: the game state
    /// lands the 300 s affect, because the affects live there, not in the storage.
    #[test]
    fn a_cloak_use_consumes_one_and_names_the_cloak() {
        let cloak = Item::new(7, CLOAK);
        let mut case = Case::new(&[(inv(3), cloak)]);
        let done = case.use_at(inv(3)).expect("a cloak is consumed");
        assert_eq!(done.kind, MoveKind::Cloaked);
        assert_eq!(
            item_records(&done),
            vec![ItemRecord::Set(gc_item_clear(inv(3)))]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert_eq!(case.at(inv(3)), None);
    }
    /// A `USE_ABILITY_UP` potion consumes one and answers `Abled`: the game state
    /// lands the apply's affect, because the affects live there, not in the storage.
    #[test]
    fn an_ability_use_consumes_one_and_names_the_ability() {
        let abled = Item::new(7, ABLED);
        let mut case = Case::new(&[(inv(3), abled)]);
        let done = case.use_at(inv(3)).expect("a potion is consumed");
        assert_eq!(done.kind, MoveKind::Abled);
        assert_eq!(
            item_records(&done),
            vec![ItemRecord::Set(gc_item_clear(inv(3)))]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert_eq!(case.at(inv(3)), None);
    }
    /// A 50801-50820 potion consumes one and answers `EuroFree`, whether
    /// `USE_ABILITY_UP` or `USE_AFFECT`: the game state lands 516 on the point.
    #[test]
    fn a_508xx_use_consumes_one_and_names_the_euro_free() {
        for vnum in [EURO_ABLED, EURO_AFFECT] {
            let euro = Item::new(7, vnum);
            let mut case = Case::new(&[(inv(3), euro)]);
            let done = case.use_at(inv(3)).expect("a potion is consumed");
            assert_eq!(done.kind, MoveKind::EuroFree, "vnum {vnum}");
            assert_eq!(
                item_records(&done),
                vec![ItemRecord::Set(gc_item_clear(inv(3)))]
            );
            assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
            assert_eq!(case.at(inv(3)), None);
        }
    }

    /// A main-switch `USE_AFFECT` item consumes one and answers `Timed`, even
    /// though its subtype is 8 like the 508xx pair's: the game state lands
    /// value(0) on the point.
    #[test]
    fn a_timed_use_consumes_one_and_names_the_timed() {
        let timed = Item::new(7, TIMED);
        let mut case = Case::new(&[(inv(3), timed)]);
        let done = case.use_at(inv(3)).expect("a use-affect item is consumed");
        assert_eq!(done.kind, MoveKind::Timed);
        assert_eq!(
            item_records(&done),
            vec![ItemRecord::Set(gc_item_clear(inv(3)))]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert_eq!(case.at(inv(3)), None);
    }

    /// A 507 premium-shop item consumes one and answers `ShopPremium`, not
    /// `Timed`: the game state runs the setter, not the point land.
    #[test]
    fn a_shop_premium_use_consumes_one_and_names_the_shop_premium() {
        let premium = Item::new(7, SHOP_PREMIUM);
        let mut case = Case::new(&[(inv(3), premium)]);
        let done = case.use_at(inv(3)).expect("a premium item is consumed");
        assert_eq!(done.kind, MoveKind::ShopPremium);
        assert_eq!(
            item_records(&done),
            vec![ItemRecord::Set(gc_item_clear(inv(3)))]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert_eq!(case.at(inv(3)), None);
    }

    /// A gold bar uses one of its stack up and names the bar, for the game state's
    /// pay.
    #[test]
    fn a_gold_bar_use_consumes_one_and_names_the_bar() {
        let bar = Item::new(7, GOLD_BAR);
        let mut case = Case::new(&[(inv(3), bar)]);
        let done = case.use_at(inv(3)).expect("a gold bar is consumed");
        assert_eq!(done.kind, MoveKind::Barred);
        assert_eq!(
            item_records(&done),
            vec![ItemRecord::Set(gc_item_clear(inv(3)))]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert_eq!(case.at(inv(3)), None);
    }

    /// A cape of courage uses one of its stack up and names the cape, for the
    /// game state's gather.
    #[test]
    fn a_cape_use_consumes_one_and_names_the_cape() {
        let held = Item::new(7, CAPE);
        let mut case = Case::new(&[(inv(3), held)]);
        let done = case.use_at(inv(3)).expect("a cape is consumed");
        assert_eq!(done.kind, MoveKind::Caped);
        assert_eq!(
            item_records(&done),
            vec![ItemRecord::Set(gc_item_clear(inv(3)))]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert_eq!(case.at(inv(3)), None);
    }

    /// A Nog pocket uses one of its stack up and names the pocket, for the
    /// game state's three rows.
    #[test]
    fn a_nog_use_consumes_one_and_names_the_pocket() {
        let pocket = Item::new(7, NOG);
        let mut case = Case::new(&[(inv(3), pocket)]);
        let done = case.use_at(inv(3)).expect("a pocket is consumed");
        assert_eq!(done.kind, MoveKind::Nogged);
        assert_eq!(
            item_records(&done),
            vec![ItemRecord::Set(gc_item_clear(inv(3)))]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert_eq!(case.at(inv(3)), None);
    }

    /// An `ITEM_USE`/`USE_SPECIAL` outside the read set still refuses: no read ran, so
    /// nothing is eaten.
    #[test]
    fn a_non_book_special_use_still_refuses() {
        let special = Item::new(7, SPECIAL);
        let mut case = Case::new(&[(inv(3), special)]);
        assert_eq!(
            case.use_refused(inv(3)),
            MoveRefused::NotPorted(Unported::UseSubType(10))
        );
    }

    /// A marriage unique refuses with `[LS;467]`: no character of this build is
    /// married (`sys.marriage` is `unused`), and the gate sits before the unique
    /// system's own arm (`G/char_item.cpp:10155-10161`).
    #[test]
    fn a_marriage_unique_refuses_its_unmarried_wearer() {
        let unique = Item::new(7, MARRIAGE_UNIQUE);
        let mut case = Case::new(&[(inv(3), unique)]);
        let unique_cell = wear_pos(EWearPositions::Unique1 as u16);
        assert_eq!(
            case.refused(inv(3), unique_cell),
            MoveRefused::UnmarriedUnique
        );
        assert_eq!(MoveRefused::UnmarriedUnique.notice(), Some("[LS;467]"));
    }

    #[test]
    fn a_sword_moved_onto_the_weapon_cell_is_worn() {
        let sword = Item::new(7, SWORD);
        let mut case = Case::new(&[(inv(3), sword.clone())]);
        let done = case.run(inv(3), wear_pos(WEAPON)).expect("equips");
        assert_eq!(done.kind, MoveKind::Equipped);
        let mut worn = sword.clone();
        worn.pos = wear_pos(WEAPON);
        assert_eq!(
            item_records(&done),
            vec![
                ItemRecord::Set(gc_item_clear(inv(3))),
                ItemRecord::Set(sword.gc_item_set(wear_pos(WEAPON), 0)),
            ]
        );
        assert_eq!(case.at(wear_pos(WEAPON)), Some(7));
        assert_eq!(case.at(inv(3)), None);
        let stored = case.items.item(7).expect("held").pos;
        assert_eq!(done.changes, vec![ItemChange::Moved { id: 7, pos: stored }]);
        assert_eq!(stored.window_type, EWindows::Equipment as u8);
        // The weapon part follows the sword, and the look goes out after the battle points.
        assert_eq!(
            case.points.parts()[common::enums::EParts::Weapon as usize],
            11
        );
        // The look rides ahead of the equip's quickslot sync (`:8525-8531`), which
        // trails the burst when no effect answers.
        let tail: Vec<&MoveRecord> = done.records.iter().rev().take(2).collect();
        assert!(
            matches!(
                tail.as_slice(),
                [MoveRecord::QuickslotSync(_), MoveRecord::Look(_)]
            ),
            "the look rides ahead of the sync: {tail:?}"
        );
        assert!(effects(&done).is_empty());
    }

    #[test]
    fn an_armour_raises_the_defence_and_taking_it_off_lowers_it_again() {
        let armour = Item::new(8, ARMOUR);
        let mut case = Case::new(&[(inv(0), armour)]);
        let bare = defence(&case.points);
        let done = case.run(inv(0), wear_pos(BODY)).expect("equips");
        assert_eq!(defence(&case.points), bare + 40);
        // The prototype's `APPLY_MOV_SPEED -2` is taken, and the look carries it and the armour.
        let main = common::enums::EParts::Main as usize;
        let armour_part = u16::try_from(ARMOUR).expect("a WORD");
        assert_eq!(case.points.limit_point(point::POINT_MOV_SPEED), 98);
        let look = looks(&done).last().copied().expect("a look");
        assert_eq!((look.moving_speed, look.attack_speed), (98, 100));
        assert_eq!(look.parts[main], armour_part);
        let points = done
            .records
            .iter()
            .filter(|record| matches!(record, MoveRecord::Point(_)))
            .count();
        assert!(points > 0, "the battle points are sent");
        assert_eq!(looks(&done).last().map(|look| look.level), Some(10));

        let done = case.run(wear_pos(BODY), inv(20)).expect("takes off");
        assert_eq!(done.kind, MoveKind::Moved);
        assert_eq!(defence(&case.points), bare);
        assert_eq!(case.points.limit_point(point::POINT_MOV_SPEED), 100);
        let look = looks(&done).last().copied().expect("a look");
        assert_eq!((look.moving_speed, look.parts[main]), (100, 0));
        let records = item_records(&done);
        assert_eq!(
            records.first(),
            Some(&ItemRecord::Set(gc_item_clear(wear_pos(BODY))))
        );
        assert_eq!(case.at(inv(20)), Some(8));
        assert_eq!(case.at(wear_pos(BODY)), None);
    }

    #[test]
    fn a_second_armour_swaps_with_the_worn_one() {
        let heavy = Item::new(9, HEAVY_ARMOUR);
        let mut case = Case::wearing(&[(BODY, Item::new(8, ARMOUR))]);
        case.items.set(inv(5), &heavy).expect("free");
        let bare = defence(&case.points) - 40;
        let done = case
            .run(inv(5), wear_pos(BODY))
            .expect_err("the wear cell is taken");
        assert_eq!(done, MoveRefused::WearCellTaken);

        // Onto any free wear cell, `EquipItem` finds the body cell itself, and swaps.
        let done = case.run(inv(5), wear_pos(WEAPON)).expect("swaps");
        assert_eq!(done.kind, MoveKind::Swapped);
        assert_eq!(case.at(wear_pos(BODY)), Some(9));
        assert_eq!(case.at(inv(5)), Some(8));
        assert_eq!(case.at(wear_pos(WEAPON)), None);
        assert_eq!(defence(&case.points), bare + 70);
        assert_eq!(
            item_records(&done),
            vec![
                ItemRecord::Set(gc_item_clear(wear_pos(BODY))),
                ItemRecord::Set(gc_item_clear(inv(5))),
                ItemRecord::Set(heavy.gc_item_set(wear_pos(BODY), 0)),
                ItemRecord::Set(Item::new(8, ARMOUR).gc_item_set(inv(5), 0)),
            ]
        );
        let moved: Vec<u32> = done
            .changes
            .iter()
            .filter_map(|change| match change {
                ItemChange::Moved { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(moved, vec![9, 8]);
    }

    #[test]
    fn a_weapon_moved_onto_a_taken_cell_is_unequipped_to_the_first_free_cell() {
        let mut case = Case::wearing(&[(WEAPON, Item::new(7, SWORD))]);
        case.items.set(inv(0), &Item::new(3, ARMOUR)).expect("free");
        let done = case.run(wear_pos(WEAPON), inv(0)).expect("unequips");
        assert_eq!(done.kind, MoveKind::Unequipped);
        assert_eq!(case.at(inv(1)), Some(7));
        assert_eq!(case.at(inv(0)), Some(3));
        assert_eq!(case.at(wear_pos(WEAPON)), None);
    }

    #[test]
    fn a_sash_unequipped_rolls_its_absorption_once() {
        // number(11, 19) with a draw of 8 is the top of the range: 8 % 9 + 11.
        let mut case = Case::wearing(&[(SASH, Item::new(4, SASH_GRADE_4))]);
        let protos = protos();
        let mut dice = Fixed(8);
        let mut gear = Gear {
            points: &mut case.points,
            protos: &protos,
            groups: &GROUPS,
            dice: &mut dice,
            recently_fought: false,
            first_use_now: None,
        };
        let mut equipper = Equipper::new(&mut case.items, &mut gear, 90);
        let sash = Item::new(4, SASH_GRADE_4);
        let sash = equipper.stored(&sash).expect("worn");
        equipper.unequip_item(&sash).expect("unequips");
        assert_eq!(case.items.item(4).expect("held").sockets[0], 19);

        let mut case = Case::wearing(&[(SASH, Item::new(4, SASH_GRADE_4))]);
        case.items.set(inv(0), &Item::new(3, SWORD)).expect("free");
        let mut dice = Fixed(5);
        let mut gear = Gear {
            points: &mut case.points,
            protos: &protos,
            groups: &GROUPS,
            dice: &mut dice,
            recently_fought: false,
            first_use_now: None,
        };
        let mut equipper = Equipper::new(&mut case.items, &mut gear, 90);
        let sash = equipper
            .items()
            .item_at(wear_pos(SASH))
            .cloned()
            .expect("worn");
        equipper.unequip_item(&sash).expect("unequips");
        let trail = equipper.into_trail();
        // number(11, 19) with a draw of 5: 5 % 9 + 11.
        let rolled = case.items.item(4).expect("held").sockets[0];
        assert_eq!(rolled, 16);
        assert!(trail.changes.contains(&ItemChange::Sockets {
            id: 4,
            sockets: [16, 0, 0, 0, 0, 0],
        }));
        // `GetEmptyInventoryEx` puts a sash in the costume bank, the fifth, before the base
        // inventory.
        let costume_bank = common::item_slots::CUSTOM_INVENTORY_SLOT_START + 5 * 180;
        assert_eq!(
            case.items.item(4).map(|item| item.pos),
            Some(inv(costume_bank))
        );
    }

    #[test]
    fn a_rolled_sash_keeps_its_absorption_and_the_lower_grades_are_fixed() {
        for (vnum, socket, expected) in [(SASH_GRADE_2, 0, 5), (SASH_GRADE_4, 12, 12)] {
            let mut sash = Item::new(4, vnum);
            sash.sockets[0] = socket;
            let mut case = Case::wearing(&[(SASH, sash)]);
            let protos = protos();
            let mut dice = Fixed(5);
            let mut gear = Gear {
                points: &mut case.points,
                protos: &protos,
                groups: &GROUPS,
                dice: &mut dice,
                recently_fought: false,
                first_use_now: None,
            };
            let mut equipper = Equipper::new(&mut case.items, &mut gear, 90);
            let worn = equipper
                .items()
                .item_at(wear_pos(SASH))
                .cloned()
                .expect("worn");
            equipper.unequip_item(&worn).expect("unequips");
            let trail = equipper.into_trail();
            assert_eq!(case.items.item(4).expect("held").sockets[0], expected);
            let socketed = trail
                .changes
                .iter()
                .any(|change| matches!(change, ItemChange::Sockets { .. }));
            assert_eq!(socketed, socket == 0, "only an unset absorption is written");
        }
    }

    #[test]
    fn a_sash_put_on_sends_the_sash_effect() {
        let mut case = Case::new(&[(inv(2), Item::new(4, SASH_GRADE_2))]);
        let done = case.run(inv(2), wear_pos(SASH)).expect("equips");
        assert_eq!(effects(&done), vec![SASH_EQUIP_EFFECT]);
    }

    /// The plain equip's quickslot sync (`G/char_item.cpp:8525-8531`): a slot naming the
    /// old cell is re-pointed at the bare wear index. The swap arm syncs nothing.
    ///
    /// The sync is vacuous for honest clients: slots name `ITEM_USE` cells (the client's
    /// add refuses anything else, `G/input_main.cpp:1095-1109`) and worn items are never
    /// `ITEM_USE` (`IsEquipable`, `G/item.cpp:1378-1397`), so no slot ever names the worn
    /// cell. The record still runs, as legacy's does.
    #[test]
    fn a_plain_equip_repoints_the_slot_at_the_wear_index_and_a_swap_syncs_nothing() {
        use crate::character::quickslot::{Quickslot, QuickslotSync, Quickslots, SyncTo};
        use common::enums::EQuickSlotType;
        let mut case = Case::new(&[(inv(5), Item::new(4, ARMOUR))]);
        let done = case.run(inv(5), wear_pos(BODY)).expect("equips");
        let syncs: Vec<QuickslotSync> = done
            .records
            .iter()
            .filter_map(|record| match record {
                MoveRecord::QuickslotSync(sync) => Some(*sync),
                _ => None,
            })
            .collect();
        assert_eq!(
            syncs,
            vec![QuickslotSync {
                from: 5,
                to: SyncTo::Cell(BODY),
            }],
            "the slot follows the wear index"
        );
        let mut slots = Quickslots::default();
        let mut out = Vec::new();
        assert!(slots.set(
            0,
            Quickslot {
                kind: EQuickSlotType::Item as u8,
                pos: 5,
            },
            &mut out,
        ));
        let applied = slots.sync(syncs[0], &case.items);
        assert_eq!(applied.len(), 1, "the re-point answers its add");
        let mut worn = Case::wearing(&[(BODY, Item::new(6, ARMOUR))]);
        worn.items
            .set(inv(5), &Item::new(4, HEAVY_ARMOUR))
            .expect("the heavy armour waits");
        let done = worn
            .run(inv(5), wear_pos(EWearPositions::Head as u16))
            .expect("swaps");
        assert!(
            done.records
                .iter()
                .all(|record| !matches!(record, MoveRecord::QuickslotSync(_))),
            "the swap arm syncs nothing"
        );
    }

    #[test]
    fn an_equip_right_after_a_fight_is_refused_with_its_notice() {
        let mut case = Case::new(&[(inv(3), Item::new(7, SWORD))]);
        case.recently_fought = true;
        assert_eq!(
            case.refused(inv(3), wear_pos(WEAPON)),
            MoveRefused::RecentlyFought
        );
        assert_eq!(MoveRefused::RecentlyFought.notice(), Some("[LS;451]"));
    }

    /// The riding gates (`G/char_item.cpp:8330-8412`): a rider puts on no wedding armour, no
    /// ride item, no body armour and no costume body; a polymorphed character wears nothing
    /// but arrows; and a walker puts the armour on.
    #[test]
    fn a_rider_and_a_morph_are_refused_their_wear() {
        for (vnum, cell, refused) in [
            (WEDDING, BODY, MoveRefused::RiderWeddingArmour),
            (MOUNT_COSTUME, MOUNT_CELL, MoveRefused::RiderRideItem),
            (ARMOUR, BODY, MoveRefused::RiderArmour),
            (COSTUME, COSTUME_BODY_CELL, MoveRefused::RiderCostume),
        ] {
            let mut case = Case::new(&[(inv(3), Item::new(7, vnum))]);
            case.points.horse_mut().set_level(25, 0);
            assert!(case.points.horse_mut().start_riding());
            assert_eq!(case.refused(inv(3), wear_pos(cell)), refused, "{vnum}");
        }
        let mut case = Case::new(&[(inv(3), Item::new(7, ARMOUR))]);
        assert!(case.points.set_polymorph(101, false));
        assert_eq!(
            case.refused(inv(3), wear_pos(BODY)),
            MoveRefused::Polymorphed
        );
        let mut case = Case::new(&[(inv(3), Item::new(7, ARMOUR))]);
        assert!(case.run(inv(3), wear_pos(BODY)).is_ok(), "a walker dresses");
    }

    /// A first-use mount wears through the use: the stamp lands first
    /// (`G/char_item.cpp:2804-2820`), then the equip checks read the started clock and
    /// the costume lands on the mount cell. A drag of the unstamped item to the cell
    /// refuses with the timer, as the wear stamps nothing; a drag of a started one
    /// lands.
    #[test]
    fn a_first_use_mount_wears_through_the_use_and_not_through_the_drag() {
        let mut case = Case::new(&[(inv(3), Item::new(7, TIMED_MOUNT))]);
        let done = case.use_at(inv(3)).expect("the use stamps and wears");
        assert_eq!(done.kind, MoveKind::Equipped);
        assert_eq!(case.at(wear_pos(MOUNT_CELL)), Some(7));
        let worn = case.items.item(7).expect("the costume is held");
        assert_eq!(worn.sockets[1], 1, "socket 1 counts the use");
        assert_ne!(worn.sockets[0], 0, "socket 0 holds the expiry");

        let mut case = Case::new(&[(inv(3), Item::new(7, TIMED_MOUNT))]);
        assert_eq!(
            case.refused(inv(3), wear_pos(MOUNT_CELL)),
            MoveRefused::NotPorted(Unported::Worn(WornSystem::Timer)),
            "an unstamped drag refuses"
        );

        let mut started = Item::new(7, TIMED_MOUNT);
        started.sockets = [1_700_060_000, 1, 0, 0, 0, 0];
        let mut case = Case::new(&[(inv(3), started)]);
        assert!(
            case.run(inv(3), wear_pos(MOUNT_CELL)).is_ok(),
            "a started drag lands"
        );
    }
    #[test]
    fn the_limits_the_sex_and_the_wedding_are_checked_before_any_change() {
        let mut case = Case::new(&[(inv(3), Item::new(7, HIGH_SWORD))]);
        assert_eq!(
            case.refused(inv(3), wear_pos(WEAPON)),
            MoveRefused::LevelTooLow
        );

        let mut case = Case::new(&[(inv(3), Item::new(7, LEVEL_TEN_SWORD))]);
        assert!(
            case.run(inv(3), wear_pos(WEAPON)).is_ok(),
            "a level limit equal to the level passes"
        );

        let mut case = Case::new(&[(inv(3), Item::new(7, MALE_ONLY))]);
        assert_eq!(case.refused(inv(3), wear_pos(BODY)), MoveRefused::WrongSex);

        let mut case = Case::wearing(&[(BODY, Item::new(8, WEDDING))]);
        case.items
            .set(inv(3), &Item::new(7, COSTUME))
            .expect("free");
        assert_eq!(
            case.refused(inv(3), wear_pos(COSTUME_BODY_CELL)),
            MoveRefused::WeddingCostume
        );

        let mut case = Case::wearing(&[(COSTUME_BODY_CELL, Item::new(8, COSTUME))]);
        case.items
            .set(inv(3), &Item::new(7, WEDDING))
            .expect("free");
        assert_eq!(
            case.refused(inv(3), wear_pos(BODY)),
            MoveRefused::WeddingArmour
        );
    }

    #[test]
    fn an_item_offered_in_a_trade_is_neither_worn_nor_used() {
        let mut case = Case::new(&[(inv(3), Item::new(7, SWORD))]);
        assert!(case.items.set_exchanging(7, true));
        assert_eq!(
            case.refused(inv(3), wear_pos(WEAPON)),
            MoveRefused::Exchanging
        );
        assert_eq!(case.use_refused(inv(3)), MoveRefused::Exchanging);
        assert_eq!(
            use_item(&mut case.items, inv(3), &RULES, None),
            Err(MoveRefused::Exchanging),
            "refused before the gear is needed"
        );
        assert!(case.items.set_exchanging(7, false));
        assert!(case.use_at(inv(3)).is_ok());
        assert_eq!(case.at(wear_pos(WEAPON)), Some(7));
    }

    #[test]
    fn a_real_time_or_grouped_unique_is_refused_as_not_ported() {
        let mut case = Case::new(&[(inv(3), Item::new(7, UNIQUE))]);
        // A plain unique wears now: the game state's wear sync arms its expiry.
        let done = case
            .run(inv(3), wear_pos(EWearPositions::Unique1 as u16))
            .expect("the unique wears");
        assert_eq!(done.kind, MoveKind::Equipped);
        // A real-time unique still refuses: its expiry is another event.
        let mut case = Case::new(&[(inv(3), Item::new(7, REAL_TIME_UNIQUE))]);
        assert_eq!(
            case.refused(inv(3), wear_pos(EWearPositions::Unique1 as u16)),
            MoveRefused::NotPorted(Unported::Worn(WornSystem::Unique))
        );
        // A unique carrying a special group still refuses.
        let mut grouped = Item::new(7, UNIQUE);
        grouped.sig_vnum = 50011;
        let mut case = Case::new(&[(inv(3), grouped)]);
        assert_eq!(
            case.refused(inv(3), wear_pos(EWearPositions::Unique1 as u16)),
            MoveRefused::NotPorted(Unported::Worn(WornSystem::Unique))
        );
    }

    #[test]
    fn the_hide_alignment_cloak_wears() {
        let mut case = Case::new(&[(inv(3), Item::new(7, HIDE_ALIGNMENT_CLOAK))]);
        // `CG_ITEM_USE` knows no unique arm (`USED_BY_WEARING` has no `ITEM_UNIQUE`),
        // so the wear goes through the move path, as the scenario's does.
        let done = case
            .run(inv(3), wear_pos(EWearPositions::Unique1 as u16))
            .expect("the cloak wears");
        assert_eq!(done.kind, MoveKind::Equipped);
        assert_eq!(case.at(wear_pos(EWearPositions::Unique1 as u16)), Some(7));
    }

    #[test]
    fn an_irremovable_worn_item_is_not_swapped() {
        let mut worn = Item::new(8, ARMOUR);
        worn.flags |= ITEM_FLAG_IRREMOVABLE;
        let mut case = Case::wearing(&[(BODY, worn)]);
        case.items
            .set(inv(5), &Item::new(9, HEAVY_ARMOUR))
            .expect("free");
        assert_eq!(
            case.refused(inv(5), wear_pos(WEAPON)),
            MoveRefused::Irremovable
        );
    }

    #[test]
    fn a_weapon_moved_onto_the_body_cell_has_no_cell_there() {
        let mut case = Case::new(&[(inv(3), Item::new(7, SWORD))]);
        // `EquipItem` finds the weapon cell itself; the body cell the client named is only
        // checked for being free.
        let done = case.run(inv(3), wear_pos(BODY)).expect("equips");
        assert_eq!(done.kind, MoveKind::Equipped);
        assert_eq!(case.at(wear_pos(WEAPON)), Some(7));
    }

    #[test]
    fn the_equip_cell_of_each_kind_follows_legacy() {
        let items = CharacterItems::new();
        let protos = protos();
        let cell = |vnum| find_equip_cell(&items, protos.get(vnum).expect("proto"));
        assert_eq!(cell(SWORD), Some(WEAPON));
        assert_eq!(cell(ARMOUR), Some(BODY));
        assert_eq!(cell(SASH_GRADE_4), Some(SASH));
        assert_eq!(cell(COSTUME), Some(COSTUME_BODY_CELL));
        assert_eq!(cell(UNIQUE), Some(EWearPositions::Unique1 as u16));
        let ring = proto(70_001, ITEM_RING, 0, 0);
        assert_eq!(
            find_equip_cell(&items, &ring),
            Some(EWearPositions::Ring1 as u16)
        );
        let worn = holding(&[(wear_pos(EWearPositions::Ring1 as u16), Item::new(1, 70_001))]);
        assert_eq!(
            find_equip_cell(&worn, &ring),
            Some(EWearPositions::Ring2 as u16)
        );
        let flagless = proto(1, ITEM_ARMOR, ARMOR_BODY, 0);
        assert_eq!(find_equip_cell(&items, &flagless), None);
    }

    #[test]
    fn the_female_races_are_one_three_four_and_six() {
        let female: Vec<u8> = (0..8).filter(|race| is_female(*race)).collect();
        assert_eq!(female, vec![1, 3, 4, 6]);
    }

    #[test]
    fn using_a_sword_puts_it_on_and_using_it_again_takes_it_off() {
        let sword = Item::new(7, SWORD);
        let mut case = Case::new(&[(inv(3), sword.clone())]);
        let on = case.use_at(inv(3)).expect("equips");
        assert_eq!(on.kind, MoveKind::Equipped);
        assert_eq!(
            item_records(&on),
            vec![
                ItemRecord::Set(gc_item_clear(inv(3))),
                ItemRecord::Set(sword.gc_item_set(wear_pos(WEAPON), 0)),
            ]
        );
        assert!(matches!(
            on.records.last(),
            Some(MoveRecord::QuickslotSync(_))
        ));
        assert_eq!(case.at(wear_pos(WEAPON)), Some(7));

        // `UnequipItem` puts it back in the first free cell, 0, not the cell it came from.
        let off = case.use_at(wear_pos(WEAPON)).expect("unequips");
        assert_eq!(off.kind, MoveKind::Unequipped);
        assert_eq!(case.at(inv(0)), Some(7));
        assert_eq!(case.at(wear_pos(WEAPON)), None);
        let records = item_records(&off);
        assert_eq!(
            records.first(),
            Some(&ItemRecord::Set(gc_item_clear(wear_pos(WEAPON))))
        );
        assert_eq!(
            records.last(),
            Some(&ItemRecord::Set(sword.gc_item_set(inv(0), 0)))
        );
        assert_eq!(
            case.points.parts()[common::enums::EParts::Weapon as usize],
            0
        );
    }

    #[test]
    fn using_a_second_armour_swaps_it_with_the_worn_one() {
        let mut case = Case::wearing(&[(BODY, Item::new(8, ARMOUR))]);
        case.items
            .set(inv(5), &Item::new(9, HEAVY_ARMOUR))
            .expect("free");
        let done = case.use_at(inv(5)).expect("swaps");
        assert_eq!(done.kind, MoveKind::Swapped);
        assert_eq!(case.at(wear_pos(BODY)), Some(9));
        assert_eq!(case.at(inv(5)), Some(8));
    }

    /// `case ITEM_QUEST` (`G/char_item.cpp:2842-2888`): a quest item passes the use checks and
    /// changes nothing, for its quest to run; the pick-up item's own arm is not ported.
    #[test]
    fn a_quest_item_passes_the_checks_and_changes_nothing() {
        let mut case = Case::new(&[(inv(3), Item::new(7, QUEST_BOOK))]);
        let items = case.items.clone();
        let done = case.use_at(inv(3)).expect("the checks pass");
        assert_eq!(done.kind, MoveKind::QuestUsed);
        assert!(done.records.is_empty());
        assert!(done.changes.is_empty());
        assert_eq!(case.items, items);
        let questing = MoveRules {
            questing: true,
            secured: false,
            ..RULES
        };
        let refused = case.use_with(inv(3), &questing);
        assert_eq!(refused, Err(MoveRefused::UsedWhileQuesting));
        let mut case = Case::new(&[(inv(3), Item::new(7, HIGH_QUEST_BOOK))]);
        assert_eq!(case.use_refused(inv(3)), MoveRefused::UseLevelTooLow);
        let mut case = Case::new(&[(inv(3), Item::new(7, PICKUP_AFFECT_ITEM))]);
        let used = case
            .use_with(inv(3), &RULES)
            .expect("70002 passes the checks");
        assert_eq!(used.kind, MoveKind::QuestUsed);
    }

    /// The lock gates the equip and the unequip last (`G/char_item.cpp:10184-10190`,
    /// `:10213-10219`).
    #[test]
    fn a_locked_character_wears_and_unwears_nothing() {
        let mut case = Case::new(&[(inv(3), Item::new(7, ARMOUR))]);
        case.points
            .point_change(point::POINT_SECURED_STATE, 1, false, false)
            .expect("the lock sets");
        assert_eq!(case.refused(inv(3), wear_pos(BODY)), MoveRefused::Secured);
        let mut case = Case::new(&[(wear_pos(BODY), Item::new(9, ARMOUR))]);
        case.points
            .point_change(point::POINT_SECURED_STATE, 1, false, false)
            .expect("the lock sets");
        assert_eq!(case.refused(wear_pos(BODY), inv(3)), MoveRefused::Secured);
    }

    /// `UseItem` (`@fixme150`, `G/char_item.cpp:7349-7354`): while a script waits nothing is
    /// used, after the job and sex checks and before `UseItemEx`'s limits.
    #[test]
    fn a_character_whose_script_waits_uses_nothing() {
        let questing = MoveRules {
            questing: true,
            secured: false,
            ..RULES
        };
        let mut case = Case::new(&[(inv(3), Item::new(7, SWORD))]);
        let items = case.items.clone();
        assert_eq!(
            case.use_with(inv(3), &questing),
            Err(MoveRefused::UsedWhileQuesting)
        );
        assert_eq!(case.items, items);
        let notice = "@@(char_item.cpp)tradus:You cannot use this item if you're using quests";
        assert_eq!(MoveRefused::UsedWhileQuesting.notice(), Some(notice));
        let mut case = Case::new(&[(inv(3), Item::new(7, MALE_ONLY))]);
        let refused = case.use_with(inv(3), &questing);
        assert_eq!(refused, Err(MoveRefused::WrongSex));
        let mut case = Case::new(&[(inv(3), Item::new(7, HIGH_SWORD))]);
        let refused = case.use_with(inv(3), &questing);
        assert_eq!(refused, Err(MoveRefused::UsedWhileQuesting));
        let mut case = Case::new(&[(inv(3), Item::new(7, SWORD))]);
        assert!(case.use_with(inv(3), &RULES).is_ok());
    }

    #[test]
    fn a_use_checks_the_job_the_sex_and_the_use_limits_before_any_change() {
        // `CanUsedBy`: a warrior may not use an item that refuses warriors.
        let mut case = Case::new(&[(inv(3), Item::new(7, SURA_REFUSED))]);
        assert_eq!(case.use_refused(inv(3)), MoveRefused::NotUsableByJob);
        assert_eq!(MoveRefused::NotUsableByJob.notice(), Some("[LS;1004]"));
        let mut case = Case::new(&[(inv(3), Item::new(7, MALE_ONLY))]);
        assert_eq!(case.use_refused(inv(3)), MoveRefused::WrongSex);
        // `FN_check_item_sex` runs in `UseItem`, before `UseItemEx`'s limits.
        let mut case = Case::new(&[(inv(3), Item::new(7, MALE_ONLY_HIGH))]);
        assert_eq!(case.use_refused(inv(3)), MoveRefused::WrongSex);
        // `UseItemEx`'s level check comes first, with its own notice, not `CanEquipNow`'s.
        let mut case = Case::new(&[(inv(3), Item::new(7, HIGH_SWORD))]);
        assert_eq!(case.use_refused(inv(3)), MoveRefused::UseLevelTooLow);
        assert_eq!(MoveRefused::UseLevelTooLow.notice(), Some("[LS;1013]"));
        let mut case = Case::new(&[(inv(3), Item::new(7, LEVEL_TEN_SWORD))]);
        assert!(
            case.use_at(inv(3)).is_ok(),
            "a limit equal to the level passes"
        );
        let mut case = Case::new(&[(inv(3), Item::new(7, CHAMPION_SWORD))]);
        assert_eq!(case.use_refused(inv(3)), MoveRefused::ChampionTooLow);
        // A first-use item stamps its clock after the checks, so a later refusal lands
        // as `Declined` with the stamps leading, in legacy's order — the stamps stay.
        let mut case = Case::new(&[(inv(3), Item::new(7, TIMED_SWORD))]);
        case.recently_fought = true;
        let stamped = case.use_at(inv(3)).expect("the stamps land, declined");
        assert_eq!(stamped.kind, MoveKind::Declined);
        assert_eq!(stamped.changes.len(), 2, "socket 0, then socket 1");
        // `EquipItem`'s own checks still run after these.
        let mut case = Case::new(&[(inv(3), Item::new(7, SWORD))]);
        case.recently_fought = true;
        assert_eq!(case.use_refused(inv(3)), MoveRefused::RecentlyFought);
    }

    #[test]
    fn a_use_of_another_type_or_from_another_window_is_not_ported() {
        let mut case = Case::new(&[(inv(3), Item::new(7, SPECIAL))]);
        assert_eq!(
            case.use_refused(inv(3)),
            MoveRefused::NotPorted(Unported::UseSubType(10))
        );
        let mut case = Case::new(&[(inv(3), Item::new(7, UNIQUE))]);
        assert_eq!(
            case.use_refused(inv(3)),
            MoveRefused::NotPorted(Unported::Use(ITEM_UNIQUE))
        );
        let mut case = Case::new(&[]);
        assert_eq!(case.use_refused(inv(3)), MoveRefused::Empty);
        let safebox = ItemPos {
            window_type: EWindows::Safebox as u8,
            cell: 0,
        };
        assert_eq!(case.use_refused(safebox), MoveRefused::InvalidSource);
        let dragon_soul = ItemPos {
            window_type: EWindows::DragonSoulInventory as u8,
            cell: 0,
        };
        assert_eq!(
            case.use_refused(dragon_soul),
            MoveRefused::NotPorted(Unported::SourceWindow(dragon_soul.window_type))
        );
        let mut items = CharacterItems::new();
        items.set(inv(3), &Item::new(7, SWORD)).expect("free");
        assert_eq!(
            use_item(&mut items, inv(3), &RULES, None),
            Err(MoveRefused::NotPorted(Unported::Equipment))
        );
    }

    #[test]
    fn a_used_potion_runs_its_arm_after_the_checks_every_use_runs() {
        let mut case = Case::new(&[
            (inv(3), Item::new(7, POTION)),
            (inv(4), Item::new(8, HIGH_POTION)),
        ]);
        case.points
            .point_change(point::POINT_HP, -200, false, false)
            .expect("a pool changes");
        let hp = case.points.hp();
        assert_eq!(case.use_refused(inv(4)), MoveRefused::UseLevelTooLow);
        let done = case.use_at(inv(3)).expect("the potion works");
        assert_eq!(done.kind, MoveKind::Used);
        assert_eq!(case.points.hp(), hp + 100);
        assert_eq!(effects(&done), [crate::character::SE_HPUP_RED]);
        assert_eq!(case.at(inv(3)), None);
    }

    #[test]
    fn a_use_from_the_belt_needs_a_belt_that_opens_the_cell() {
        let first = BELT_INVENTORY_SLOT_START;
        let mut case = Case::new(&[(inv(first + 2), Item::new(7, SWORD))]);
        assert_eq!(case.use_refused(inv(first + 2)), MoveRefused::NoBeltWorn);
        let grade_two = MoveRules {
            belt_grade: Some(2),
            ..RULES
        };
        // Belt cell 2 needs grade 4.
        assert_eq!(
            case.use_with(inv(first + 2), &grade_two),
            Err(MoveRefused::BeltCellLocked)
        );
        let grade_four = MoveRules {
            belt_grade: Some(4),
            ..RULES
        };
        let done = case.use_with(inv(first + 2), &grade_four).expect("equips");
        assert_eq!(done.kind, MoveKind::Equipped);
        // `UseItemEx`'s limits run before its belt check.
        let mut case = Case::new(&[(inv(first + 2), Item::new(7, CHAMPION_SWORD))]);
        assert_eq!(
            case.use_refused(inv(first + 2)),
            MoveRefused::ChampionTooLow
        );
    }

    #[test]
    fn a_use_that_cannot_free_the_costume_weapon_keeps_the_notice_it_sent() {
        // The worn costume is for another weapon sub type, so the sword needs it off first, and
        // with no usable inventory cell it cannot come off.
        let mut case = Case::wearing(&[(
            EWearPositions::CostumeWeapon as u16,
            Item::new(5, COSTUME_BLADE),
        )]);
        case.items.set(inv(3), &Item::new(7, SWORD)).expect("free");
        let no_room = MoveRules {
            usable_cells: 0,
            ..RULES
        };
        let items = case.items.clone();
        let done = case.use_with(inv(3), &no_room).expect("a notice was sent");
        assert_eq!(done.kind, MoveKind::Declined);
        assert_eq!(
            done.records.first(),
            Some(&MoveRecord::Notice(
                MoveRefused::NoRoomToUnequip.notice().expect("[1130]")
            ))
        );
        assert!(done.changes.is_empty());
        assert_eq!(case.items, items);
    }

    /// The wear cell a position names: the equipment's flat cells, and nothing else.
    #[test]
    fn a_position_names_a_wear_cell_only_when_it_is_one() {
        assert_eq!(wear_of(wear_pos(0)), Some(0));
        assert_eq!(wear_of(wear_pos(WEAR_MAX_NUM - 1)), Some(WEAR_MAX_NUM - 1));
        assert_eq!(wear_of(wear_pos(WEAR_MAX_NUM)), None);
        // The base inventory and the belt are not wear cells.
        assert_eq!(wear_of(inv(0)), None);
        assert_eq!(wear_of(inv(INVENTORY_MAX_NUM - 1)), None);
        // Another window is not the equipment either.
        assert_eq!(
            wear_of(ItemPos::new(EWindows::Equipment as u8, INVENTORY_MAX_NUM)),
            Some(0)
        );
        assert_eq!(
            wear_of(ItemPos::new(EWindows::DragonSoulInventory as u8, 0)),
            None
        );
    }

    /// `wear_full_set` strips the six first cells and the shield, then puts each
    /// made piece where `FindEquipCell` says, skipping what cannot go on.
    #[test]
    fn a_full_set_strips_then_kits_in_order() {
        use gamedata::item_kind::ARMOR_BODY;
        let protos = ItemProtos::from_rows(vec![
            proto(SWORD, ITEM_WEAPON, 0, WEARABLE_WEAPON),
            proto(ARMOUR, ITEM_ARMOR, ARMOR_BODY, WEARABLE_BODY),
        ]);
        let mut items = holding(&[
            (wear_pos(BODY), Item::new(3, ARMOUR)),
            (inv(0), Item::new(4, SWORD)),
        ]);
        let mut points = warrior();
        let mut dice = Fixed(5);
        let mut gear = Gear {
            points: &mut points,
            protos: &protos,
            groups: &GROUPS,
            dice: &mut dice,
            recently_fought: false,
            first_use_now: None,
        };
        let mut next = 10;
        let done = wear_full_set(&mut items, &mut gear, 90, &[ARMOUR, SWORD, 999], |vnum| {
            protos.get(vnum)?;
            next += 1;
            Some(Item::new(next, vnum))
        });
        assert_eq!(done.kind, MoveKind::FullSet);
        // The old body went to the first free base cell; the fresh pieces went on.
        assert!(items
            .item_at(wear_pos(BODY))
            .is_some_and(|item| item.vnum == ARMOUR));
        assert!(items
            .item_at(wear_pos(WEAPON))
            .is_some_and(|item| item.vnum == SWORD));
        assert!(items.item_at(inv(1)).is_some_and(|item| item.id == 3));
        let created = done
            .changes
            .iter()
            .filter(|change| matches!(change, ItemChange::Created(_)))
            .count();
        assert_eq!(created, 2, "the unknown vnum makes nothing");
    }

    /// The unique and ring tail of the equip effect switch
    /// (`G/char_item.cpp:8592-8604`): a ring with a `SIGVnum` sends the attr
    /// group's effect path, zero-padded; without a group it sends nothing.
    #[test]
    fn a_ring_with_a_sig_sends_its_groups_effect_path() {
        use gamedata::special_item_group::{
            GroupType, SpecialAttrGroup, SpecialItemGroup, SpecialItemGroups, SpecialItemInfo,
        };
        let bag = SpecialItemGroup {
            vnum: 900,
            group_type: GroupType::Special,
            items: vec![SpecialItemInfo {
                vnum: RING,
                count: 42,
                rare: 0,
            }],
            totals: vec![1],
        };
        let attr = SpecialAttrGroup {
            vnum: 42,
            effect_file: b"ring.effect".to_vec(),
            attrs: Vec::new(),
        };
        let mut groups = SpecialItemGroups::default();
        groups.special.insert(900, bag);
        groups.attr.insert(42, attr);
        let equip = |groups: &SpecialItemGroups| {
            let mut ring = Item::new(7, RING);
            ring.sig_vnum = 900;
            let mut case = Case::new(&[(inv(3), ring)]);
            let protos = protos();
            let mut dice = Fixed(5);
            let mut gear = Gear {
                points: &mut case.points,
                protos: &protos,
                groups,
                dice: &mut dice,
                recently_fought: false,
                first_use_now: None,
            };
            let mut equipper = Equipper::new(&mut case.items, &mut gear, 90);
            let item = equipper.items().item(7).cloned().expect("held");
            equipper.equip_item(&item).expect("equips");
            equipper.into_trail()
        };
        let trail = equip(&groups);
        let mut path = [0u8; 128];
        path[..11].copy_from_slice(b"ring.effect");
        assert!(trail.records.contains(&MoveRecord::SpecificEffect(path)));
        assert!(equip(&GROUPS)
            .records
            .iter()
            .all(|record| !matches!(record, MoveRecord::SpecificEffect(_))));
    }
}
