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
//! does not have ([`WornSystem`]): a dragon soul stone, an aura, mount or pet costume, an
//! `ITEM_UNIQUE` or `WEARABLE_UNIQUE` item, a timed item, an accessory with stones, an item
//! with immunities, and a bonus type whose point is not ported. The item load refuses the same
//! items in a wear cell, so everything worn can be taken off.
//!
//! # One Defect this does not reproduce
//!
//! `SwapItem` takes the worn item off before `EquipTo` and `AddToCharacter` run, and neither
//! reports a failure it can meet (`G/char_item.cpp:8237-8250`). Here every check runs first,
//! so a swap either happens whole or changes nothing.
//!
//! Unreachable here, and so not written: riding and polymorph, the unique unstack, the
//! wedding checks `__FIX_COSTUM_NUNTA_PESTE_COSTUM_NORMAL__` repeats (the `ENABLE_WEDDING_FIX`
//! ones return first), the special item group effect (a dangling `else` binds it to the sash
//! arm), `REAL_TIME_FIRST_USE`, the mount quest, the aura window, `IsSecured`,
//! `IsExchanging` and the quickslot sync.

use common::enums::EWearPositions;
use common::item_slots::{
    EWindows, BELT_INVENTORY_SLOT_END, BELT_INVENTORY_SLOT_START, INVENTORY_MAX_NUM,
};
use common::point_slot as point;
use gamedata::item_custom_category::{is_custom_category, CATEGORY_NUM};
use gamedata::item_kind::{
    COSTUME_AURA, COSTUME_BODY, COSTUME_HAIR, COSTUME_MOUNT, COSTUME_PET, COSTUME_SASH,
    COSTUME_WEAPON, ITEM_ANTIFLAG_ASSASSIN, ITEM_ANTIFLAG_FEMALE, ITEM_ANTIFLAG_MALE,
    ITEM_ANTIFLAG_SHAMAN, ITEM_ANTIFLAG_SURA, ITEM_ANTIFLAG_WARRIOR, ITEM_ARMOR, ITEM_BELT,
    ITEM_COSTUME, ITEM_DS, ITEM_PICK, ITEM_RING, ITEM_ROD, ITEM_SPECIAL_DS, ITEM_TALISMAN,
    ITEM_TOTEM, ITEM_UNIQUE, ITEM_WEAPON, LIMIT_CHAMPION, LIMIT_CON, LIMIT_DEX, LIMIT_INT,
    LIMIT_LEVEL, LIMIT_REAL_TIME, LIMIT_REAL_TIME_START_FIRST_USE, LIMIT_STR, WEARABLE_ABILITY,
    WEARABLE_ARROW, WEARABLE_BODY, WEARABLE_DARK, WEARABLE_EAR, WEARABLE_EARTH, WEARABLE_ELEC,
    WEARABLE_FIRE, WEARABLE_FOOTS, WEARABLE_GLOVE, WEARABLE_HEAD, WEARABLE_ICE, WEARABLE_NECK,
    WEARABLE_SHIELD, WEARABLE_UNIQUE, WEARABLE_WEAPON, WEARABLE_WIND, WEARABLE_WRIST,
};
use gamedata::item_proto::{ItemProto, ItemProtos};
use protocol::item_pos::ItemPos;

use super::dice::{number, Dice};
use super::equipment::{
    accessory_socket_grade, is_set_item, item_applies, removal_applies, Equipment, Worn, PARTS,
};
use super::item_move::{ItemChange, ItemRecord, MoveRecord, MoveRefused, Unported};
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
    /// The draw a sash's absorption is rolled from.
    pub dice: &'a mut dyn Dice,
    /// Whether the character attacked or used a skill in the last 1.5 s
    /// (`G/char_item.cpp:8471-8477`).
    pub recently_fought: bool,
}

impl core::fmt::Debug for Gear<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Gear")
            .field("recently_fought", &self.recently_fought)
            .finish_non_exhaustive()
    }
}

/// What `UpdatePacket` sends of a character whose look changed (`G/char.cpp:1277-1340`).
///
/// The other fields of `GC_CHARACTER_UPDATE` (the state flags, the affects, the guild, the
/// alignment, the PK mode, the mount and the premium) come from systems this build does not
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
}

/// A system that wearing an item starts, which this build has not ported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WornSystem {
    /// A dragon soul stone: `EquipTo` activates it in the dragon soul deck.
    DragonSoul,
    /// An aura costume: its drain, its armour and its booster timer.
    Aura,
    /// A mount costume: `EquipTo` summons the mount.
    Mount,
    /// A pet costume: `EquipTo` summons the pet.
    Pet,
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
            Self::Mount => formatter.write_str("the mount costume"),
            Self::Pet => formatter.write_str("the pet costume"),
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
/// the unique, wear-timer, accessory and aura-booster timers, and summons a mount or pet
/// costume (`G/item.cpp:1458-1487`); `OnAfterCreatedItem` starts the real-time timer
/// (`:2775-2781`). `BuffOnAttr_AddBuffsFromItem` does nothing, because only the two bonus types
/// this build refuses fill its table.
#[must_use]
pub fn worn_system_not_ported(
    item: &Item,
    proto: &ItemProto,
    protos: &ItemProtos,
) -> Option<WornSystem> {
    if proto.item_type == ITEM_COSTUME {
        match proto.sub_type {
            COSTUME_AURA => return Some(WornSystem::Aura),
            COSTUME_MOUNT => return Some(WornSystem::Mount),
            COSTUME_PET => return Some(WornSystem::Pet),
            _ => {}
        }
    }
    if proto.item_type == ITEM_UNIQUE {
        return Some(WornSystem::Unique);
    }
    let timed = proto
        .limits
        .iter()
        .any(|limit| [LIMIT_REAL_TIME, LIMIT_REAL_TIME_START_FIRST_USE].contains(&limit.kind));
    if timed || proto.timer_based_on_wear.is_some() {
        return Some(WornSystem::Timer);
    }
    if accessory_socket_grade(item, proto) > 0 {
        return Some(WornSystem::AccessoryTimer);
    }
    if proto.immune_flags != 0 {
        return Some(WornSystem::Immunity);
    }
    item_applies(item, proto, protos)
        .into_iter()
        .find(|(apply, _)| !apply_is_ported(*apply))
        .map(|(apply, _)| WornSystem::Apply(apply))
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

/// The item a wear cell holds.
fn worn_at(items: &CharacterItems, wear: u16) -> Option<Item> {
    items.item_at(wear_pos(wear)).cloned()
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
    fn look(&self) -> CharacterLook {
        let points = &*self.gear.points;
        let byte = |kind: usize| u8::try_from(points.limit_point(kind) & 0xff).unwrap_or(0);
        CharacterLook {
            parts: points.parts(),
            moving_speed: byte(point::POINT_MOV_SPEED),
            attack_speed: byte(point::POINT_ATT_SPEED),
            level: u32::from(points.level()),
            conqueror_level: u32::from(points.conqueror_level()),
            refine_element_type: Equipment::of(self.items, self.gear.protos).refine_element_type(),
        }
    }

    fn push_points(&mut self, records: Vec<super::points::PointRecord>) {
        self.trail
            .records
            .extend(records.into_iter().map(MoveRecord::Point));
    }

    /// `CanEquipNow` (`G/char_item.cpp:10060-10193`).
    pub(crate) fn can_equip_now(&self, item: &Item, proto: &ItemProto) -> Result<(), MoveRefused> {
        let points = &*self.gear.points;
        let job_flag = [
            ITEM_ANTIFLAG_WARRIOR,
            ITEM_ANTIFLAG_ASSASSIN,
            ITEM_ANTIFLAG_SURA,
            ITEM_ANTIFLAG_SHAMAN,
        ]
        .get(usize::from(race_to_job(points.race())))
        .copied()
        .unwrap_or(0);
        if proto.anti_flags & job_flag != 0 {
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
        if proto.wear_flags & WEARABLE_UNIQUE != 0 {
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
        Ok(())
    }

    /// The applies, then the parts switch, of `ModifyPoints` (`G/item.cpp:781-1375`).
    fn modify_points(&mut self, wear: u16, item: &Item, proto: &'g ItemProto, add: bool) {
        let protos = self.gear.protos;
        let applies = item_applies(item, proto, protos);
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
        let change = Equipment::of(self.items, protos).part_change(
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
        let equipment = Equipment::of(self.items, self.gear.protos);
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
        let sash = proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_SASH;
        let rolled = sash && item.sockets[SASH_ABSORPTION_SOCKET] == 0;
        if rolled {
            item.sockets[SASH_ABSORPTION_SOCKET] = match proto.values[0] {
                2 => 5,
                3 => 10,
                4 => number(self.gear.dice, 11, 19),
                _ => 1,
            };
        }
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
        let female = is_female(self.gear.points.race());
        let refused_sex = if female {
            ITEM_ANTIFLAG_FEMALE
        } else {
            ITEM_ANTIFLAG_MALE
        };
        if proto.anti_flags & refused_sex != 0 {
            return Err(MoveRefused::WrongSex);
        }
        if wear != EWearPositions::Arrow as u16 && self.gear.recently_fought {
            return Err(MoveRefused::RecentlyFought);
        }
        if proto.item_type == ITEM_DS || proto.item_type == ITEM_SPECIAL_DS {
            return Err(MoveRefused::NotPorted(Unported::Worn(
                WornSystem::DragonSoul,
            )));
        }
        if let Some(system) = worn_system_not_ported(item, proto, self.gear.protos) {
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
                self.put_on(wear, &item)?;
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
}

#[cfg(test)]
mod tests {
    use gamedata::item_kind::{ARMOR_BODY, ITEM_ANTIFLAG_MALE, LIMIT_LEVEL};
    use gamedata::item_proto::ItemValue;

    use super::*;
    use crate::character::item_move::{move_item, MoveDone, MoveKind, MoveRequest, MoveRules};
    use crate::character::item_move::{MoveFacts, MoveRecord};
    use crate::character::points::PointsRow;

    const INV: u8 = EWindows::Inventory as u8;
    const WEAPON: u16 = EWearPositions::Weapon as u16;
    const BODY: u16 = EWearPositions::Body as u16;
    const SASH: u16 = EWearPositions::CostumeSash as u16;
    const COSTUME_BODY_CELL: u16 = EWearPositions::CostumeBody as u16;

    const SWORD: u32 = 11;
    const ARMOUR: u32 = 11_210;
    const HEAVY_ARMOUR: u32 = 11_220;
    const HIGH_SWORD: u32 = 21;
    const SASH_GRADE_4: u32 = 85_004;
    const SASH_GRADE_2: u32 = 85_002;
    const COSTUME: u32 = 41_001;
    const WEDDING: u32 = 11_901;
    const MALE_ONLY: u32 = 11_230;
    const UNIQUE: u32 = 71_001;
    const LEVEL_TEN_SWORD: u32 = 31;
    const UNIQUE_FLAGGED: u32 = 11_240;

    const RULES: MoveRules = MoveRules {
        count_limit: 200,
        usable_cells: 90,
        belt_grade: None,
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
        let mut male_only = proto(MALE_ONLY, ITEM_ARMOR, ARMOR_BODY, WEARABLE_BODY);
        male_only.anti_flags = ITEM_ANTIFLAG_MALE;
        ItemProtos::from_rows(vec![
            proto(SWORD, ITEM_WEAPON, 0, WEARABLE_WEAPON),
            armour,
            heavy,
            high,
            grade_4,
            grade_2,
            proto(COSTUME, ITEM_COSTUME, COSTUME_BODY, 0),
            proto(WEDDING, ITEM_ARMOR, ARMOR_BODY, WEARABLE_BODY),
            male_only,
            proto(UNIQUE, ITEM_UNIQUE, 0, WEARABLE_UNIQUE),
            level_ten,
            proto(UNIQUE_FLAGGED, ITEM_ARMOR, ARMOR_BODY, WEARABLE_UNIQUE),
        ])
    }

    fn warrior() -> Points {
        Points::load(&PointsRow {
            race: 0,
            level: 10,
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
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
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
                dice: &mut dice,
                recently_fought: self.recently_fought,
            };
            let request = MoveRequest { from, to, count: 0 };
            let facts = MoveFacts {
                categories: Vec::new(),
                belt_eligible: false,
                dragon_soul: false,
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
        let last = done.records.last().expect("records");
        assert!(
            matches!(last, MoveRecord::Look(_)),
            "the look is last: {last:?}"
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
            dice: &mut dice,
            recently_fought: false,
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
            dice: &mut dice,
            recently_fought: false,
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
                dice: &mut dice,
                recently_fought: false,
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
    fn a_unique_item_is_refused_as_not_ported() {
        let mut case = Case::new(&[(inv(3), Item::new(7, UNIQUE))]);
        assert_eq!(
            case.refused(inv(3), wear_pos(EWearPositions::Unique1 as u16)),
            MoveRefused::NotPorted(Unported::Worn(WornSystem::Unique))
        );
        // `WEARABLE_UNIQUE` on an item of another type is refused by `CanEquipNow` too.
        let mut case = Case::new(&[(inv(3), Item::new(7, UNIQUE_FLAGGED))]);
        assert_eq!(
            case.refused(inv(3), wear_pos(EWearPositions::Unique1 as u16)),
            MoveRefused::NotPorted(Unported::Worn(WornSystem::Unique))
        );
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
}
