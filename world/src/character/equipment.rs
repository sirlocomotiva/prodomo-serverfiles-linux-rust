//! What a character's worn items give it: the bonuses each item applies, the parts the client
//! draws, the set bonus, and the armour the battle points add.
//!
//! | legacy | here |
//! |---|---|
//! | `CItem::ModifyPoints(true)`, the apply half (`item.cpp:781-1155`) | [`item_applies`] |
//! | `CItem::ModifyPoints(true)`, the parts switch (`item.cpp:1156-1337`) | [`Equipment::parts`] |
//! | the worn loop of `ComputePoints` (`char.cpp:3070-3080`) | [`Equipment::modify_points`] |
//! | the set bonus of `ComputePoints` (`char.cpp:3082-3110`) | [`Equipment::set_bonus_applies`] |
//! | the armour sum of `ComputeBattlePoints` (`char.cpp:2776-2794`) | [`Equipment::armour`] |
//! | `IsNewSetNeedRefresh` (`char.cpp:11747-11763`) | [`is_set_item`] |
//!
//! An apply is an `APPLY_*` type and a value, in the order legacy calls `ApplyPoint` with
//! them; `crate::character::Points::apply_point` carries one out. A worn item is the item in
//! the inventory window at `INVENTORY_MAX_NUM` plus its wear cell, which is where legacy's
//! `GetWear` finds it.
//!
//! # Where the Rewrite differs from legacy, and why
//!
//! - **A rarity outside the table adds nothing.** Legacy reads socket 4 of an armour as an
//!   index into the three-entry table `{0, 20, 50}` (`item.cpp:938-944` and `1146-1155`)
//!   without a bound, so a value above 2, or a negative one in the first use, reads past the
//!   table. That is a Defect, and the Rewrite adds nothing for such a value.
//! - **A refine element past the sixth adds no enchantment.** Legacy applies an element's
//!   bonus as `APPLY_ENCHANT_ELECT + (type - 1)` (`item.cpp:912` and `1142`), so a type above
//!   6 names an unrelated apply type (7 is `APPLY_SUNGMA_STR`) or, past 129, none. That is a
//!   Defect, and the Rewrite grants nothing for it.
//! - **Arithmetic does not wrap.** Legacy multiplies in 32-bit `long`. The Rewrite computes
//!   in 64 bits, or 128 for the sash scale, and saturates when it narrows to 32.
//!
//! # Not ported
//!
//! The aura costume's drain (`item.cpp:983-1019` and `1045-1073`) and its armour
//! (`char.cpp:2796-2819`), and the dragon soul alchemy bonus (`item.cpp:1092-1113`): the item
//! load refuses aura costumes and dragon soul stones, so none of them is ever worn. The
//! `ITEM_UNIQUE` and `ITEM_RING` arms apply the bonuses of a special attribute group, and the
//! owner's `special_item_group.txt` has none (`gamedata::special_item_group`'s test
//! `the_owners_file_is_one_group_of_twenty_nine_rows`), so both apply nothing.

use common::enums::{EParts, EWearPositions};
use common::item_slots::{EWindows, INVENTORY_MAX_NUM, WEAR_MAX_NUM};
use gamedata::item_kind::{
    ARMOR_BODY, ARMOR_EAR, ARMOR_FOOTS, ARMOR_HEAD, ARMOR_NECK, ARMOR_SHIELD, ARMOR_WRIST,
    COSTUME_AURA, COSTUME_BODY, COSTUME_HAIR, COSTUME_MOUNT, COSTUME_SASH, COSTUME_WEAPON,
    ITEM_ARMOR, ITEM_BELT, ITEM_COSTUME, ITEM_METIN, ITEM_PICK, ITEM_ROD, ITEM_WEAPON,
};
use gamedata::item_proto::{ItemProto, ItemProtos};
use protocol::item_pos::ItemPos;

use super::apply::{
    APPLY_ATTBONUS_BOSS, APPLY_ATTBONUS_HUMAN, APPLY_ATTBONUS_MONSTER, APPLY_ATT_GRADE_BONUS,
    APPLY_DEF_GRADE_BONUS, APPLY_ENCHANT_ELECT, APPLY_MAGIC_ATT_GRADE, APPLY_MAGIC_DEF_GRADE,
    APPLY_MAX_HP, APPLY_NONE, APPLY_SKILL,
};
use super::CharacterItems;
use crate::item::Item;

/// The wear cells, `WEAR_MAX_NUM`.
const WEARS: usize = WEAR_MAX_NUM as usize;

/// The parts a character shows, `PART_MAX_NUM`.
pub const PARTS: usize = EParts::MaxNum as usize;

/// `ITEM_ACCESSORY_SOCKET_MAX_NUM`: the highest grade an accessory's sockets reach.
const ACCESSORY_SOCKET_MAX_NUM: i32 = 3;

/// `aiAccessorySocketEffectivePct` (`constants.cpp:1160-1163`): how much of each of the first
/// two applies an accessory's grade adds, in percent.
const ACCESSORY_SOCKET_EFFECTIVE_PCT: [i64; 4] = [0, 10, 20, 40];

/// `SASH_ABSORPTION_SOCKET`: the socket holding the share of the absorbed item, in percent.
const SASH_ABSORPTION_SOCKET: usize = 0;

/// `SASH_ABSORBED_SOCKET`: the socket holding the absorbed item's vnum.
const SASH_ABSORBED_SOCKET: usize = 1;

/// `SASH_EFFECT_FROM_ABS`: the share from which a sash shows its glowing look.
const SASH_EFFECT_FROM_ABS: i32 = 19;

/// The socket an armour keeps its rarity in.
const RARITY_SOCKET: usize = 4;

/// The four items whose attributes legacy skips (`CItemVnumHelper`, `VnumHelper.h:60-69`): the
/// Ramadan moon ring, the Halloween candy, the happiness ring and the love pendant.
const ATTRIBUTES_SKIPPED: [u32; 4] = [71_135, 71_136, 71_143, 71_145];

/// One set of `m_mapNewSetBonus` (`constants.cpp:205-276`).
struct NewSet {
    /// Each wear cell of the set and the vnum it must hold.
    slots: [(u16, u32); 8],
    /// How many bonuses a count of worn pieces gives, indexed by the count.
    bonus_count: [usize; 9],
    /// The bonuses, in the order the count gives them.
    bonuses: [(u8, i32); 5],
}

/// The bonus counts both sets use.
const SET_BONUS_COUNT: [usize; 9] = [0, 0, 1, 2, 3, 3, 4, 4, 5];

/// `m_mapNewSetBonus`, in the order a `std::map` keyed by name visits it.
const NEW_SETS: [NewSet; 2] = [
    // "SetBonus - 1"
    NewSet {
        slots: [
            (EWearPositions::Weapon as u16, 19),
            (EWearPositions::Body as u16, 11_209),
            (EWearPositions::Head as u16, 12_209),
            (EWearPositions::Shield as u16, 13_009),
            (EWearPositions::Wrist as u16, 14_009),
            (EWearPositions::Foots as u16, 15_009),
            (EWearPositions::Neck as u16, 16_009),
            (EWearPositions::Ear as u16, 17_009),
        ],
        bonus_count: SET_BONUS_COUNT,
        bonuses: [
            (APPLY_MAX_HP, 500),
            (APPLY_ATTBONUS_MONSTER, 3),
            (APPLY_ATTBONUS_BOSS, 3),
            (APPLY_ATTBONUS_BOSS, 3),
            (APPLY_ATTBONUS_HUMAN, 3),
        ],
    },
    // "SetBonus - 2"
    NewSet {
        slots: [
            (EWearPositions::Weapon as u16, 3_159),
            (EWearPositions::Body as u16, 11_299),
            (EWearPositions::Head as u16, 12_249),
            (EWearPositions::Shield as u16, 13_049),
            (EWearPositions::Wrist as u16, 14_109),
            (EWearPositions::Foots as u16, 15_189),
            (EWearPositions::Neck as u16, 16_109),
            (EWearPositions::Ear as u16, 17_109),
        ],
        bonus_count: SET_BONUS_COUNT,
        bonuses: [
            (APPLY_MAX_HP, 3_000),
            (APPLY_ATTBONUS_MONSTER, 20),
            (APPLY_ATTBONUS_BOSS, 20),
            (APPLY_ATTBONUS_BOSS, 20),
            (APPLY_ATTBONUS_HUMAN, 20),
        ],
    },
];

/// One worn item and its proto.
#[derive(Debug, Clone, Copy)]
pub struct Worn<'a> {
    /// The item.
    pub item: &'a Item,
    /// Its proto.
    pub proto: &'a ItemProto,
}

/// The items a character wears, by wear cell.
#[derive(Debug, Clone)]
pub struct Equipment<'a> {
    cells: [Option<Worn<'a>>; WEARS],
    protos: Option<&'a ItemProtos>,
}

impl Default for Equipment<'_> {
    /// Nothing worn.
    fn default() -> Self {
        Self {
            cells: [None; WEARS],
            protos: None,
        }
    }
}

impl<'a> Equipment<'a> {
    /// The items `items` holds in the wear cells, each with its proto. An item whose proto
    /// is missing is not worn; the item load refuses such an item before it gets here.
    #[must_use]
    pub fn of(items: &'a CharacterItems, protos: &'a ItemProtos) -> Self {
        let mut cells = [None; WEARS];
        for (wear, cell) in (0..WEAR_MAX_NUM).zip(cells.iter_mut()) {
            let pos = ItemPos::new(EWindows::Inventory as u8, INVENTORY_MAX_NUM + wear);
            *cell = items
                .item_at(pos)
                .and_then(|item| protos.get(item.vnum).map(|proto| Worn { item, proto }));
        }
        Self {
            cells,
            protos: Some(protos),
        }
    }

    /// `GetWear(wear)`: the item in a wear cell.
    #[must_use]
    pub fn wear(&self, wear: u16) -> Option<Worn<'a>> {
        self.cells.get(usize::from(wear)).copied().flatten()
    }

    /// Every worn item with its wear cell, in wear-cell order.
    pub fn worn(&self) -> impl Iterator<Item = (u16, Worn<'a>)> + '_ {
        (0..WEAR_MAX_NUM)
            .zip(self.cells.iter())
            .filter_map(|(wear, cell)| cell.map(|worn| (wear, worn)))
    }

    /// The worn loop of `ComputePoints`: each worn item in wear-cell order, with the applies
    /// its `ModifyPoints(true)` makes.
    #[must_use]
    pub fn modify_points(&self) -> Vec<(Worn<'a>, Vec<(u8, i32)>)> {
        let Some(protos) = self.protos else {
            return Vec::new();
        };
        self.worn()
            .map(|(_, worn)| (worn, item_applies(worn.item, worn.proto, protos)))
            .collect()
    }

    /// The armour `ComputeBattlePoints` adds: for each worn body, head, foot or shield
    /// armour, its proto's `value1` plus twice its `value5`.
    #[must_use]
    pub fn armour(&self) -> i32 {
        let sum: i64 = self
            .worn()
            .map(|(_, worn)| worn.proto)
            .filter(|proto| {
                proto.item_type == ITEM_ARMOR
                    && [ARMOR_BODY, ARMOR_HEAD, ARMOR_FOOTS, ARMOR_SHIELD].contains(&proto.sub_type)
            })
            .map(|proto| i64::from(proto.values[1]) + 2 * i64::from(proto.values[5]))
            .sum();
        narrow(sum)
    }

    /// The parts after `ComputePoints`: `base` is the character's base part (`bBasePart`)
    /// and `current` the parts before the computation.
    ///
    /// `ComputePoints` sets each part to `GetOriginalPart` (`char.cpp:2917-2926`), which is
    /// the base part for the body, the current part for the weapon, hair and sash, and 0 for
    /// the head and the aura (`char.cpp:5424-5462`). The worn loop then runs each worn item's
    /// parts switch in wear-cell order. `SetPart` stores a `WORD`, so each value keeps its low
    /// 16 bits.
    #[must_use]
    pub fn parts(&self, base: u16, current: [u16; PARTS]) -> [u16; PARTS] {
        let mut parts = [0; PARTS];
        parts[EParts::Main as usize] = base;
        for part in [EParts::Weapon, EParts::Hair, EParts::Sash] {
            parts[part as usize] = current[part as usize];
        }
        for (wear, worn) in self.worn() {
            if let Some((part, value)) = self.part_of(wear, worn) {
                parts[part as usize] = word(value);
            }
        }
        parts
    }

    /// The part one worn item's `ModifyPoints(true)` sets, if any.
    fn part_of(&self, wear: u16, worn: Worn<'a>) -> Option<(EParts, u32)> {
        let Worn { item, proto } = worn;
        let look = if item.transmutation == 0 {
            item.vnum
        } else {
            item.transmutation
        };
        match proto.item_type {
            ITEM_PICK | ITEM_ROD => {
                (wear == EWearPositions::Weapon as u16).then_some((EParts::Weapon, item.vnum))
            }
            ITEM_WEAPON => {
                if self.wear(EWearPositions::CostumeWeapon as u16).is_some() {
                    return None;
                }
                (wear == EWearPositions::Weapon as u16).then_some((EParts::Weapon, look))
            }
            ITEM_ARMOR => {
                if self.wear(EWearPositions::CostumeBody as u16).is_some() {
                    return None;
                }
                (proto.sub_type == ARMOR_BODY).then_some((EParts::Main, look))
            }
            ITEM_COSTUME => match proto.sub_type {
                COSTUME_BODY => Some((EParts::Main, look)),
                COSTUME_HAIR => {
                    let from = if item.transmutation == 0 {
                        Some(proto)
                    } else {
                        self.protos
                            .and_then(|protos| protos.get(item.transmutation))
                            .or(Some(proto))
                    };
                    from.map(|from| {
                        (
                            EParts::Hair,
                            u32::from_ne_bytes(from.values[3].to_ne_bytes()),
                        )
                    })
                }
                COSTUME_AURA => (wear == EWearPositions::CostumeAura as u16)
                    .then_some((EParts::Aura, item.vnum)),
                COSTUME_SASH => {
                    let mut value = look.wrapping_sub(85_000);
                    if item.sockets[SASH_ABSORPTION_SOCKET] >= SASH_EFFECT_FROM_ABS {
                        value = value.wrapping_add(2_000);
                    }
                    Some((EParts::Sash, value))
                }
                COSTUME_WEAPON => Some((EParts::Weapon, look)),
                _ => None,
            },
            _ => None,
        }
    }

    /// The set bonus of `ComputePoints`: for each set, the bonuses the count of its worn
    /// pieces gives, in the order legacy applies them.
    #[must_use]
    pub fn set_bonus_applies(&self) -> Vec<(u8, i32)> {
        let mut applies = Vec::new();
        for set in &NEW_SETS {
            let count = set
                .slots
                .iter()
                .filter(|(wear, vnum)| self.wear(*wear).is_some_and(|w| w.item.vnum == *vnum))
                .count();
            let given = set.bonus_count.get(count).copied().unwrap_or(0);
            applies.extend(set.bonuses.iter().take(given));
        }
        applies
    }
}

/// `IsNewSetNeedRefresh`: whether an item of this vnum is a piece of a set, so that wearing or
/// removing it recomputes the points.
#[must_use]
pub fn is_set_item(vnum: u32) -> bool {
    NEW_SETS
        .iter()
        .any(|set| set.slots.iter().any(|(_, piece)| *piece == vnum))
}

/// The applies `ModifyPoints(true)` makes for one worn item, in the order legacy makes them.
///
/// `protos` resolves the stones in the item's sockets and the item a sash has absorbed. An
/// entry may carry `APPLY_NONE`, which changes nothing, as legacy calls `ApplyPoint` with it.
#[must_use]
pub fn item_applies(item: &Item, proto: &ItemProto, protos: &ItemProtos) -> Vec<(u8, i32)> {
    let mut applies = Vec::new();
    let accessory_grade = accessory_socket_grade(item, proto);
    if !is_accessory_for_socket(proto) {
        socket_stones(item, protos, &mut applies);
    }
    let sash = proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_SASH;
    let absorbed = if sash {
        protos.get(u32::from_ne_bytes(
            item.sockets[SASH_ABSORBED_SOCKET].to_ne_bytes(),
        ))
    } else {
        None
    };
    let absorption = item.sockets[SASH_ABSORPTION_SOCKET];
    if sash && item.sockets[SASH_ABSORBED_SOCKET] != 0 {
        if let Some(absorbed) = absorbed {
            sash_grades(item, absorbed, absorption, &mut applies);
        }
    }
    proto_applies(item, proto, absorbed, accessory_grade, &mut applies);
    if !ATTRIBUTES_SKIPPED.contains(&item.vnum) {
        for attribute in &item.attributes {
            if attribute.b_type == 0 {
                continue;
            }
            let mut value = i32::from(attribute.s_value);
            if sash {
                value = scale(i64::from(value), absorption);
                if attribute.s_value > 0 && value <= 0 {
                    value += 1;
                }
            }
            applies.push((attribute.b_type, value));
        }
    }
    if proto.item_type == ITEM_WEAPON && refine_type(item) != 0 {
        let attack = refine_attack(item);
        if attack > 0 {
            applies.push((APPLY_ATT_GRADE_BONUS, attack));
        }
        let bonus = refine_bonus(item);
        if bonus > 0 {
            if let Some(enchant) = enchant(refine_type(item)) {
                applies.push((enchant, bonus));
            }
        }
    }
    if proto.item_type == ITEM_ARMOR && item.sockets[RARITY_SOCKET] > 0 {
        let defence = i64::from(proto.values[1]) + 2 * i64::from(proto.values[5]);
        let pct = rarity_pct(item.sockets[RARITY_SOCKET]);
        applies.push((APPLY_DEF_GRADE_BONUS, narrow(defence * pct / 100)));
    }
    applies
}

/// `GetAccessorySocketGrade` (`item.cpp:2684-2692`): how many of an accessory's sockets hold a
/// stone, which is socket 0 kept between 0 and the grade socket 1 allows. 0 for any item that
/// is not an accessory, whose sockets hold stones instead.
///
/// A worn accessory with a grade above 0 starts the timer that takes its stones away one by
/// one (`StartAccessorySocketExpireEvent`, `item.cpp:2339-2365`).
#[must_use]
pub fn accessory_socket_grade(item: &Item, proto: &ItemProto) -> i32 {
    if !is_accessory_for_socket(proto) {
        return 0;
    }
    let max_grade = item.sockets[1].clamp(0, ACCESSORY_SOCKET_MAX_NUM);
    item.sockets[0].clamp(0, max_grade)
}

/// `IsAccessoryForSocket` (`item.cpp:2273-2277`): a bracelet, necklace or earring, or a belt.
fn is_accessory_for_socket(proto: &ItemProto) -> bool {
    (proto.item_type == ITEM_ARMOR
        && [ARMOR_WRIST, ARMOR_NECK, ARMOR_EAR].contains(&proto.sub_type))
        || proto.item_type == ITEM_BELT
}

/// The applies of the stones in a non-accessory's sockets (`item.cpp:788-820`).
fn socket_stones(item: &Item, protos: &ItemProtos, applies: &mut Vec<(u8, i32)>) {
    for socket in item.sockets {
        let vnum = u32::from_ne_bytes(socket.to_ne_bytes());
        if vnum <= 2 {
            continue;
        }
        let Some(stone) = protos.get(vnum) else {
            continue;
        };
        if stone.item_type != ITEM_METIN {
            continue;
        }
        for apply in &stone.applies {
            let kind = apply_type(apply.kind);
            if kind != APPLY_NONE {
                applies.push((kind, apply.value));
            }
        }
    }
}

/// The grades a sash takes from the item it absorbed (`item.cpp:829-918`).
fn sash_grades(item: &Item, absorbed: &ItemProto, absorption: i32, applies: &mut Vec<(u8, i32)>) {
    let v = absorbed.values.map(i64::from);
    if absorbed.item_type == ITEM_ARMOR && absorbed.sub_type == ARMOR_BODY {
        let mut defence = scale(v[1] + 2 * v[5], absorption);
        if v[1] > 0 || v[5] > 0 {
            defence = defence.saturating_add(1);
        }
        applies.push((APPLY_DEF_GRADE_BONUS, defence));
        let mut magic_defence = scale(v[0], absorption);
        if v[0] > 0 {
            magic_defence = magic_defence.saturating_add(1);
        }
        applies.push((APPLY_MAGIC_DEF_GRADE, magic_defence));
    } else if absorbed.item_type == ITEM_WEAPON {
        let mut attack = if v[3] > v[4] {
            v[3] + v[5]
        } else {
            v[4] + v[5]
        };
        let own_type = refine_type(item);
        if own_type != 0 && refine_attack(item) > 0 {
            attack += i64::from(refine_attack(item));
        }
        let mut attack = scale(attack, absorption);
        if v[3] > 0 || v[4] > 0 {
            attack = attack.saturating_add(1);
        }
        applies.push((APPLY_ATT_GRADE_BONUS, attack));
        let magic_attack = if v[1] > v[2] {
            v[1] + v[5]
        } else {
            v[2] + v[5]
        };
        let mut magic_attack = scale(magic_attack, absorption);
        if v[1] > 0 || v[2] > 0 {
            magic_attack = magic_attack.saturating_add(1);
        }
        applies.push((APPLY_MAGIC_ATT_GRADE, magic_attack));
        if own_type != 0 {
            let bonus = scale(i64::from(refine_bonus(item)), absorption);
            if bonus > 0 {
                if let Some(enchant) = enchant(own_type) {
                    applies.push((enchant, bonus));
                }
            }
        }
    }
}

/// The proto's three applies, with the rarity, the sash and the accessory grade folded in
/// (`item.cpp:921-980`).
fn proto_applies(
    item: &Item,
    proto: &ItemProto,
    absorbed: Option<&ItemProto>,
    accessory_grade: i32,
    applies: &mut Vec<(u8, i32)>,
) {
    let sash = proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_SASH;
    let mount = proto.item_type == ITEM_COSTUME && proto.sub_type == COSTUME_MOUNT;
    for (i, apply) in proto.applies.iter().enumerate() {
        let mut kind = apply_type(apply.kind);
        // Legacy tests the type and the sub-type apart, so a `NONE` apply of any costume, or
        // of any item whose sub-type number is `COSTUME_SASH`, is still applied.
        if kind == APPLY_NONE && proto.item_type != ITEM_COSTUME && proto.sub_type != COSTUME_SASH {
            continue;
        }
        if mount {
            continue;
        }
        let mut value = i64::from(apply.value);
        let rarity = item.sockets[RARITY_SOCKET];
        if rarity != 0 && proto.item_type == ITEM_ARMOR {
            value += value * rarity_pct(rarity) / 100;
        }
        if sash {
            let Some(absorbed) = absorbed else {
                continue;
            };
            let from = absorbed.applies[i];
            kind = apply_type(from.kind);
            if kind == APPLY_NONE || from.value < 0 {
                continue;
            }
            let mut scaled = scale(i64::from(from.value), item.sockets[SASH_ABSORPTION_SOCKET]);
            if from.value > 0 && scaled <= 0 {
                scaled += 1;
            }
            value = i64::from(scaled);
        }
        if kind != APPLY_SKILL && accessory_grade != 0 && i < proto.applies.len() - 1 {
            let grade = usize::try_from(accessory_grade).unwrap_or(0);
            let share = value * ACCESSORY_SOCKET_EFFECTIVE_PCT[grade] / 100;
            value += share.max(i64::from(accessory_grade));
        }
        applies.push((kind, narrow(value)));
    }
}

/// An apply type the proto reader keeps as an `i32`, as the `BYTE` legacy stores. The reader
/// refuses a type outside the table, so the fallback is never taken.
fn apply_type(kind: i32) -> u8 {
    u8::try_from(kind).unwrap_or(APPLY_NONE)
}

/// The share of an armour's grade its rarity adds, in percent: `{0, 20, 50}` by rarity, and
/// nothing for a rarity the table does not have.
fn rarity_pct(rarity: i32) -> i64 {
    match rarity {
        1 => 20,
        2 => 50,
        _ => 0,
    }
}

/// `(long)((double)(value * pct) / 100 + .5)`: the share of an absorbed value a sash keeps.
///
/// The quotient of an integer by 100 is never within rounding of a half, so the double
/// arithmetic is the integer quotient of `value * pct + 50` by 100, truncated toward zero.
fn scale(value: i64, pct: i32) -> i32 {
    let scaled = (i128::from(value) * i128::from(pct) + 50) / 100;
    i32::try_from(scaled).unwrap_or(if scaled < 0 { i32::MIN } else { i32::MAX })
}

/// `GetRefineElementType` (`item.h:97`): the element the item was refined with, 0 for none.
fn refine_type(item: &Item) -> u32 {
    item.refine_element / 100_000_000
}

/// `GetRefineElementBonusValue` (`item.h:99`).
fn refine_bonus(item: &Item) -> i32 {
    i32::try_from(item.refine_element / 100_000 % 100).unwrap_or(0)
}

/// `GetRefineElementAttackValue` (`item.h:100`).
fn refine_attack(item: &Item) -> i32 {
    i32::try_from(item.refine_element / 1_000 % 100).unwrap_or(0)
}

/// The enchantment an element grants: `APPLY_ENCHANT_ELECT` for the first through
/// `APPLY_ENCHANT_DARK` for the sixth, and none for any other.
fn enchant(element: u32) -> Option<u8> {
    let element = u8::try_from(element).ok()?;
    (1..=6)
        .contains(&element)
        .then(|| APPLY_ENCHANT_ELECT + element - 1)
}

/// The `WORD` `SetPart` keeps of a value.
fn word(value: u32) -> u16 {
    let [low, high, ..] = value.to_le_bytes();
    u16::from_le_bytes([low, high])
}

/// Narrow a 64-bit total to the 32 bits legacy keeps it in, saturating rather than wrapping.
fn narrow(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gamedata::item_proto::ItemValue;
    use protocol::gc_entity::ENTITY_PART_NUM;
    use protocol::gc_item_window::ItemAttribute;

    use super::*;
    use crate::character::apply::APPLY_MAX_SP;
    use crate::character::legacy_source::{auto_numbered, legacy, span, strip_comments, switches};

    const INV: u8 = EWindows::Inventory as u8;
    const BODY: u16 = EWearPositions::Body as u16;
    const HEAD: u16 = EWearPositions::Head as u16;
    const FOOTS: u16 = EWearPositions::Foots as u16;
    const WRIST: u16 = EWearPositions::Wrist as u16;
    const WEAPON: u16 = EWearPositions::Weapon as u16;
    const NECK: u16 = EWearPositions::Neck as u16;
    const EAR: u16 = EWearPositions::Ear as u16;
    const UNIQUE1: u16 = EWearPositions::Unique1 as u16;
    const SHIELD: u16 = EWearPositions::Shield as u16;
    const COSTUME_BODY_CELL: u16 = EWearPositions::CostumeBody as u16;
    const COSTUME_HAIR_CELL: u16 = EWearPositions::CostumeHair as u16;
    const COSTUME_WEAPON_CELL: u16 = EWearPositions::CostumeWeapon as u16;
    const COSTUME_SASH_CELL: u16 = EWearPositions::CostumeSash as u16;
    const COSTUME_AURA_CELL: u16 = EWearPositions::CostumeAura as u16;

    /// A metin stone, an absorbed body armour and an absorbed weapon for the fixtures.
    const STONE: u32 = 28_101;
    const ABSORBED_ARMOUR: u32 = 11_290;
    const ABSORBED_WEAPON: u32 = 190;

    fn proto(vnum: u32, item_type: i32, sub_type: i32) -> ItemProto {
        ItemProto::for_category_rule(vnum, item_type, sub_type)
    }

    fn applying(mut proto: ItemProto, applies: [(u8, i32); 3]) -> ItemProto {
        proto.applies = applies.map(|(kind, value)| ItemValue {
            kind: i32::from(kind),
            value,
        });
        proto
    }

    fn valued(mut proto: ItemProto, values: [i32; 6]) -> ItemProto {
        proto.values = values;
        proto
    }

    fn item(id: u32, vnum: u32) -> Item {
        Item::new(id, vnum)
    }

    fn socketed(mut item: Item, sockets: [i32; 6]) -> Item {
        item.sockets = sockets;
        item
    }

    fn wearing(worn: &[(u16, &Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (wear, item) in worn {
            let pos = ItemPos::new(INV, INVENTORY_MAX_NUM + wear);
            items.set(pos, item).expect("the wear cell is free");
        }
        items
    }

    /// The value of an enumerator written `NAME = number` in a legacy header.
    fn enumerator(path: &str, name: &str) -> i64 {
        let text = strip_comments(&legacy(path));
        let values: Vec<i64> = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix(name))
            .filter_map(|rest| rest.trim_start().strip_prefix('='))
            .map(|value| {
                let value = value.trim().trim_end_matches(',').trim();
                value.parse().unwrap_or_else(|_| panic!("{name} = {value}"))
            })
            .collect();
        assert_eq!(values.len(), 1, "{name} is defined once in {path}");
        values[0]
    }

    fn tokens(line: &str) -> Vec<&str> {
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|token| !token.is_empty())
            .collect()
    }

    #[test]
    fn the_switches_this_module_follows_are_compiled_in() {
        let defined = switches();
        for switch in [
            "__SASH_SYSTEM__",
            "__CHANGELOOK_SYSTEM__",
            "ENABLE_WEAPON_COSTUME_SYSTEM",
            "ENABLE_REFINE_ELEMENT",
            "ENABLE_MOUNT_COSTUME_SYSTEM",
            "__NEW_SET_BONUS__",
            "__AURA_SYSTEM__",
            "ENABLE_DRAGONSOUL_ALCHEMY_PLUS",
        ] {
            assert!(defined.contains(switch), "{switch}");
        }
        // A control: a switch the source tests and never defines.
        assert!(!defined.contains("_IMPROVED_PACKET_ENCRYPTION_"));
    }

    #[test]
    fn the_wear_cells_and_parts_are_numbered_as_this_build_compiles_them() {
        let wears = auto_numbered("common/length.h", "EWearPositions", "WEAR_", "WEAR_MAX");
        assert!(wears.len() <= WEARS, "{} wear cells", wears.len());
        let at = |name: &str| {
            let index = wears.iter().position(|wear| wear == name);
            index
                .and_then(|i| u16::try_from(i).ok())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        for (name, wear) in [
            ("WEAR_BODY", BODY),
            ("WEAR_HEAD", HEAD),
            ("WEAR_FOOTS", FOOTS),
            ("WEAR_WRIST", WRIST),
            ("WEAR_WEAPON", WEAPON),
            ("WEAR_NECK", NECK),
            ("WEAR_EAR", EAR),
            ("WEAR_UNIQUE1", UNIQUE1),
            ("WEAR_SHIELD", SHIELD),
            ("WEAR_COSTUME_BODY", COSTUME_BODY_CELL),
            ("WEAR_COSTUME_HAIR", COSTUME_HAIR_CELL),
            ("WEAR_COSTUME_WEAPON", COSTUME_WEAPON_CELL),
            ("WEAR_COSTUME_SASH", COSTUME_SASH_CELL),
            ("WEAR_COSTUME_AURA", COSTUME_AURA_CELL),
        ] {
            assert_eq!(at(name), wear, "{name}");
        }
        let parts = auto_numbered("common/length.h", "EParts", "PART_", "PART_MAX_NUM");
        assert_eq!(
            parts,
            [
                "PART_MAIN",
                "PART_WEAPON",
                "PART_HEAD",
                "PART_HAIR",
                "PART_SASH",
                "PART_AURA"
            ]
        );
        assert_eq!(parts.len(), PARTS);
        assert_eq!(PARTS, ENTITY_PART_NUM);
        let ours = [
            EParts::Main,
            EParts::Weapon,
            EParts::Head,
            EParts::Hair,
            EParts::Sash,
            EParts::Aura,
        ];
        for (index, part) in ours.into_iter().enumerate() {
            assert_eq!(part as usize, index);
        }
    }

    #[test]
    fn the_constants_are_the_legacy_ones() {
        let sash = "common/item_length.h";
        assert_eq!(enumerator(sash, "SASH_ABSORPTION_SOCKET"), 0);
        assert_eq!(enumerator(sash, "SASH_ABSORBED_SOCKET"), 1);
        assert_eq!(enumerator(sash, "SASH_EFFECT_FROM_ABS"), 19);
        assert_eq!(SASH_ABSORPTION_SOCKET, 0);
        assert_eq!(SASH_ABSORBED_SOCKET, 1);
        assert_eq!(SASH_EFFECT_FROM_ABS, 19);
        assert_eq!(enumerator(sash, "ITEM_APPLY_MAX_NUM"), 3);
        assert_eq!(
            enumerator("game/constants.h", "ITEM_ACCESSORY_SOCKET_MAX_NUM"),
            3
        );
        assert_eq!(ACCESSORY_SOCKET_MAX_NUM, 3);

        let text = strip_comments(&legacy("game/constants.cpp"));
        let table = span(&text, "aiAccessorySocketEffectivePct[", "};").join(" ");
        let (_, body) = table.split_once('{').expect("the table has a body");
        let pct: Vec<i64> = tokens(body)
            .into_iter()
            .map(|token| token.parse().expect("a number"))
            .collect();
        assert_eq!(pct, ACCESSORY_SOCKET_EFFECTIVE_PCT);

        let helper = strip_comments(&legacy("common/VnumHelper.h"));
        let skipped: Vec<u32> = [
            "IsRamadanMoonRing",
            "IsHalloweenCandy",
            "IsHappinessRing",
            "IsLovePendant",
        ]
        .into_iter()
        .map(|name| {
            let line = helper
                .lines()
                .find(|line| line.contains(&format!("{name}(DWORD vnum)")))
                .unwrap_or_else(|| panic!("no {name}"));
            let (_, body) = line.split_once("return ").expect("one comparison");
            let (vnum, _) = body.split_once("==").expect("one comparison");
            vnum.trim().parse().expect("a vnum")
        })
        .collect();
        assert_eq!(skipped, ATTRIBUTES_SKIPPED);
    }

    #[test]
    fn the_set_bonus_table_is_the_legacy_one() {
        struct Parsed {
            name: String,
            slots: Vec<(u16, u32)>,
            counts: Vec<(usize, usize)>,
            bonuses: Vec<(u8, u8, i32)>,
        }
        let wears = auto_numbered("common/length.h", "EWearPositions", "WEAR_", "WEAR_MAX");
        let apply = |name: &str| match name {
            "APPLY_MAX_HP" => APPLY_MAX_HP,
            "APPLY_ATTBONUS_MONSTER" => APPLY_ATTBONUS_MONSTER,
            "APPLY_ATTBONUS_BOSS" => APPLY_ATTBONUS_BOSS,
            "APPLY_ATTBONUS_HUMAN" => APPLY_ATTBONUS_HUMAN,
            other => panic!("an apply this table does not know: {other}"),
        };
        let text = strip_comments(&legacy("game/constants.cpp"));
        let mut sets: Vec<Parsed> = Vec::new();
        for line in span(&text, "m_mapNewSetBonus = {", "};")
            .into_iter()
            .skip(1)
        {
            let words = tokens(line);
            if let Some((_, rest)) = line.split_once('"') {
                let (name, _) = rest.split_once('"').expect("a quoted name");
                sets.push(Parsed {
                    name: name.to_owned(),
                    slots: Vec::new(),
                    counts: Vec::new(),
                    bonuses: Vec::new(),
                });
                continue;
            }
            let Some(set) = sets.last_mut() else {
                assert!(words.is_empty(), "{line}");
                continue;
            };
            match words.as_slice() {
                [wear, vnum] if wear.starts_with("WEAR_") => {
                    let index = wears.iter().position(|w| w == wear).expect("a wear cell");
                    let wear = u16::try_from(index).expect("a wear cell");
                    set.slots.push((wear, vnum.parse().expect("a vnum")));
                }
                [count, given] => {
                    let count = count.parse().expect("a count");
                    set.counts.push((count, given.parse().expect("a count")));
                }
                [index, "std", "make_pair", kind, value] => {
                    let index = index.parse().expect("an index");
                    set.bonuses
                        .push((index, apply(kind), value.parse().expect("a value")));
                }
                [] | ["TNewSetBonus"] => {}
                other => panic!("a line this parser does not model: {other:?}"),
            }
        }
        // `std::map` visits its string keys in byte order.
        let mut names: Vec<&str> = sets.iter().map(|set| set.name.as_str()).collect();
        let parsed_order = names.clone();
        names.sort_unstable();
        assert_eq!(
            parsed_order, names,
            "the source lists the sets in map order"
        );
        assert_eq!(names, ["SetBonus - 1", "SetBonus - 2"]);
        assert_eq!(sets.len(), NEW_SETS.len());
        for (parsed, ours) in sets.iter().zip(&NEW_SETS) {
            assert_eq!(parsed.slots, ours.slots, "{}", parsed.name);
            let counts: BTreeMap<usize, usize> = parsed.counts.iter().copied().collect();
            assert_eq!(counts.len(), parsed.counts.len(), "no count repeats");
            let expected: BTreeMap<usize, usize> =
                ours.bonus_count.iter().copied().enumerate().collect();
            assert_eq!(counts, expected, "{}", parsed.name);
            let bonuses: Vec<(u8, u8, i32)> = (1..)
                .zip(ours.bonuses)
                .map(|(index, (kind, value))| (index, kind, value))
                .collect();
            assert_eq!(parsed.bonuses, bonuses, "{}", parsed.name);
        }
    }

    #[test]
    fn a_stone_in_a_socket_applies_its_protos_applies() {
        let stone = applying(
            proto(STONE, ITEM_METIN, 0),
            [
                (APPLY_MAX_HP, 0x0102),
                (APPLY_NONE, 5),
                (APPLY_SKILL, 0x0080_0301),
            ],
        );
        // A socket that holds an armour's vnum is not a stone and adds nothing.
        let not_a_stone = applying(
            proto(12_010, ITEM_ARMOR, ARMOR_HEAD),
            [(APPLY_MAX_HP, 99); 3],
        );
        let sword = proto(10, ITEM_WEAPON, 0);
        let protos = ItemProtos::from_rows(vec![stone, not_a_stone, sword.clone()]);
        let stones = |sockets| item_applies(&socketed(item(1, 10), sockets), &sword, &protos);
        // 0, 1 and 2 are the empty and blocked socket marks, a vnum with no proto adds nothing,
        // and -1 reads as the vnum 4294967295, which has none.
        let sockets = [1, STONE.try_into().expect("a vnum"), 2, -1, 12_010, 0];
        assert_eq!(
            stones(sockets),
            [(APPLY_MAX_HP, 0x0102), (APPLY_SKILL, 0x0080_0301)]
        );
        let twice = [28_101, 0, 28_101, 0, 0, 0];
        assert_eq!(stones(twice).len(), 4, "each socket applies its stone");
    }

    #[test]
    fn an_accessory_grade_raises_its_first_two_applies() {
        let neck = applying(
            proto(16_010, ITEM_ARMOR, ARMOR_NECK),
            [
                (APPLY_MAX_HP, 0x0203),
                (APPLY_ATTBONUS_HUMAN, 7),
                (APPLY_MAX_SP, 0x0105),
            ],
        );
        let stone = applying(proto(STONE, ITEM_METIN, 0), [(APPLY_MAX_HP, 1_000); 3]);
        let protos = ItemProtos::from_rows(vec![neck.clone(), stone]);
        let with = |sockets| item_applies(&socketed(item(1, 16_010), sockets), &neck, &protos);
        // Grade 2 of at most 3: 515 + max(2, 515 * 20 / 100) and 7 + max(2, 7 * 20 / 100). The
        // third apply keeps its value, and socket 2 is the accessory's timer, not a stone.
        let stone = STONE.try_into().expect("a vnum");
        assert_eq!(
            with([2, 3, stone, 0, 0, 0]),
            [
                (APPLY_MAX_HP, 618),
                (APPLY_ATTBONUS_HUMAN, 9),
                (APPLY_MAX_SP, 0x0105)
            ]
        );
        // The grade is socket 0 bounded by socket 1, and socket 1 is bounded by 3.
        assert_eq!(with([3, 1, 0, 0, 0, 0])[0], (APPLY_MAX_HP, 515 + 51));
        assert_eq!(with([9, 9, 0, 0, 0, 0])[0], (APPLY_MAX_HP, 515 + 206));
        assert_eq!(with([-4, 3, 0, 0, 0, 0])[0], (APPLY_MAX_HP, 515));
        assert_eq!(with([0; 6])[1], (APPLY_ATTBONUS_HUMAN, 7));
        // A belt is an accessory as well.
        let belt = applying(proto(18_000, ITEM_BELT, 0), [(APPLY_MAX_HP, 100); 3]);
        let protos = ItemProtos::from_rows(vec![belt.clone()]);
        let belted = socketed(item(2, 18_000), [1, 1, 0, 0, 0, 0]);
        assert_eq!(
            item_applies(&belted, &belt, &protos),
            [
                (APPLY_MAX_HP, 110),
                (APPLY_MAX_HP, 110),
                (APPLY_MAX_HP, 100)
            ]
        );
        // A skill apply is never graded. An earring skips its NONE applies.
        let applies = [(APPLY_SKILL, 0x0080_0301), (APPLY_NONE, 0), (APPLY_NONE, 0)];
        let ear = applying(proto(17_020, ITEM_ARMOR, ARMOR_EAR), applies);
        let wrist = applying(proto(14_020, ITEM_ARMOR, ARMOR_WRIST), applies);
        let protos = ItemProtos::from_rows(vec![ear.clone(), wrist.clone()]);
        let graded = socketed(item(3, 17_020), [3, 3, 0, 0, 0, 0]);
        assert_eq!(
            item_applies(&graded, &ear, &protos),
            [(APPLY_SKILL, 0x0080_0301)]
        );
        // A bracelet's sub-type number is `COSTUME_SASH`'s, so legacy's test keeps its NONE
        // applies, and the first one is graded. Applying NONE changes nothing.
        let graded = socketed(item(4, 14_020), [3, 3, 0, 0, 0, 0]);
        assert_eq!(ARMOR_WRIST, COSTUME_SASH);
        assert_eq!(
            item_applies(&graded, &wrist, &protos),
            [(APPLY_SKILL, 0x0080_0301), (APPLY_NONE, 3), (APPLY_NONE, 0)]
        );
    }

    #[test]
    fn a_sash_applies_its_share_of_an_absorbed_armour() {
        let armour = applying(
            valued(
                proto(ABSORBED_ARMOUR, ITEM_ARMOR, ARMOR_BODY),
                [0x0105, 0x0203, 0, 0, 0, 0x0011],
            ),
            [
                (APPLY_MAX_HP, 1_000),
                (APPLY_NONE, 9),
                (APPLY_ATTBONUS_HUMAN, -5),
            ],
        );
        let sash = applying(
            proto(85_001, ITEM_COSTUME, COSTUME_SASH),
            [(APPLY_NONE, 0), (APPLY_MAX_SP, 77), (APPLY_NONE, 0)],
        );
        let protos = ItemProtos::from_rows(vec![armour, sash.clone()]);
        let absorbed = ABSORBED_ARMOUR.try_into().expect("a vnum");
        let mut worn = socketed(item(1, 85_001), [25, absorbed, 0, 0, 0, 0]);
        worn.attributes[0] = ItemAttribute::new(APPLY_ATTBONUS_MONSTER, 3);
        worn.attributes[2] = ItemAttribute::new(APPLY_MAX_HP, 1);
        worn.attributes[3] = ItemAttribute::new(APPLY_ATTBONUS_HUMAN, -300);
        worn.attributes[4] = ItemAttribute::new(0, 99);
        assert_eq!(
            item_applies(&worn, &sash, &protos),
            [
                // (515 + 2 * 17) * 25% is 137.25, rounded to 137, plus 1.
                (APPLY_DEF_GRADE_BONUS, 138),
                // 261 * 25% is 65.25, rounded to 65, plus 1.
                (APPLY_MAGIC_DEF_GRADE, 66),
                // The absorbed item's applies replace the sash's own; a NONE one and a
                // negative one are skipped.
                (APPLY_MAX_HP, 250),
                // 3 * 25% is 0.75, rounded to 1.
                (APPLY_ATTBONUS_MONSTER, 1),
                // 0.25 rounds to 0, and a positive value never scales to nothing.
                (APPLY_MAX_HP, 1),
                // -75 + 0.5 truncates toward zero.
                (APPLY_ATTBONUS_HUMAN, -74),
            ]
        );
        // With nothing absorbed, a sash applies only its attributes.
        let mut empty = worn.clone();
        empty.sockets[SASH_ABSORBED_SOCKET] = 0;
        assert_eq!(item_applies(&empty, &sash, &protos).len(), 3);
        let mut lost = worn;
        lost.sockets[SASH_ABSORBED_SOCKET] = 99_999;
        assert_eq!(item_applies(&lost, &sash, &protos).len(), 3);
    }

    #[test]
    fn a_sash_applies_its_share_of_an_absorbed_weapon_and_its_own_element() {
        let weapon = valued(
            proto(ABSORBED_WEAPON, ITEM_WEAPON, 0),
            [0, 0x0120, 0x0110, 0x0210, 0x0220, 0x0030],
        );
        let sash = proto(85_004, ITEM_COSTUME, COSTUME_SASH);
        let protos = ItemProtos::from_rows(vec![weapon, sash.clone()]);
        let absorbed = ABSORBED_WEAPON.try_into().expect("a vnum");
        let mut worn = socketed(item(1, 85_004), [25, absorbed, 0, 0, 0, 0]);
        // Element 2 with a bonus of 15 and an attack of 40.
        worn.refine_element = 201_540_000;
        assert_eq!(
            item_applies(&worn, &sash, &protos),
            [
                // v3 is not above v4, so 544 + 48, plus the element's 40: 632 * 25% is 158.
                (APPLY_ATT_GRADE_BONUS, 159),
                // v1 is above v2, so 288 + 48: 336 * 25% is 84.
                (APPLY_MAGIC_ATT_GRADE, 85),
                // 15 * 25% is 3.75, rounded to 4, as the second element's enchantment.
                (APPLY_ENCHANT_ELECT + 1, 4),
            ]
        );
        worn.refine_element = 0;
        assert_eq!(
            item_applies(&worn, &sash, &protos),
            [(APPLY_ATT_GRADE_BONUS, 149), (APPLY_MAGIC_ATT_GRADE, 85)]
        );
        // With v3 above v4, the attack is v3 + v5 and the magic attack v2 + v5.
        let other = valued(
            proto(ABSORBED_WEAPON + 1, ITEM_WEAPON, 0),
            [0, 0x0110, 0x0120, 0x0230, 0x0220, 0x0030],
        );
        let protos = ItemProtos::from_rows(vec![other, sash.clone()]);
        let absorbed = (ABSORBED_WEAPON + 1).try_into().expect("a vnum");
        let worn = socketed(item(2, 85_004), [25, absorbed, 0, 0, 0, 0]);
        assert_eq!(
            item_applies(&worn, &sash, &protos),
            // 560 + 48 is 608, and 288 + 48 is 336: a quarter of each, plus one.
            [(APPLY_ATT_GRADE_BONUS, 153), (APPLY_MAGIC_ATT_GRADE, 85)]
        );
    }

    #[test]
    fn an_armours_rarity_raises_its_applies_and_its_defence() {
        let armour = applying(
            valued(
                proto(11_210, ITEM_ARMOR, ARMOR_BODY),
                [0, 0x0123, 0, 0, 0, 0x0011],
            ),
            [
                (APPLY_MAX_HP, 0x0201),
                (APPLY_NONE, 0),
                (APPLY_ATTBONUS_HUMAN, 0x0105),
            ],
        );
        let protos = ItemProtos::from_rows(vec![armour.clone()]);
        let rare = |rarity| {
            let worn = socketed(item(1, 11_210), [0, 0, 0, 0, rarity, 0]);
            item_applies(&worn, &armour, &protos)
        };
        // Rarity 2 adds half: 513 + 256 and 261 + 130, then (291 + 2 * 17) * 50%.
        assert_eq!(
            rare(2),
            [
                (APPLY_MAX_HP, 769),
                (APPLY_ATTBONUS_HUMAN, 391),
                (APPLY_DEF_GRADE_BONUS, 162)
            ]
        );
        assert_eq!(
            rare(1),
            [
                (APPLY_MAX_HP, 615),
                (APPLY_ATTBONUS_HUMAN, 313),
                (APPLY_DEF_GRADE_BONUS, 65)
            ]
        );
        // A rarity past the table adds nothing, and still applies a defence of 0.
        assert_eq!(
            rare(3),
            [
                (APPLY_MAX_HP, 513),
                (APPLY_ATTBONUS_HUMAN, 261),
                (APPLY_DEF_GRADE_BONUS, 0)
            ]
        );
        assert_eq!(rare(-1), [(APPLY_MAX_HP, 513), (APPLY_ATTBONUS_HUMAN, 261)]);
        assert_eq!(rare(0), rare(-1));
        // Only an armour is rare: a weapon keeps its applies and has no defence to raise.
        let sword = applying(
            valued(proto(10, ITEM_WEAPON, 0), [0, 0x0123, 0, 0, 0, 0x0011]),
            [(APPLY_MAX_HP, 0x0201), (APPLY_NONE, 0), (APPLY_NONE, 0)],
        );
        let protos = ItemProtos::from_rows(vec![sword.clone()]);
        let worn = socketed(item(2, 10), [0, 0, 0, 0, 2, 0]);
        assert_eq!(item_applies(&worn, &sword, &protos), [(APPLY_MAX_HP, 513)]);
    }

    #[test]
    fn a_costume_applies_even_its_none_applies_and_a_mount_applies_none() {
        let costume = applying(
            proto(41_001, ITEM_COSTUME, COSTUME_BODY),
            [(APPLY_NONE, 5), (APPLY_MAX_HP, 0x0102), (APPLY_NONE, 0)],
        );
        let mount = applying(
            proto(71_224, ITEM_COSTUME, COSTUME_MOUNT),
            [(APPLY_MAX_HP, 0x0102); 3],
        );
        let sword = applying(
            proto(10, ITEM_WEAPON, 0),
            [(APPLY_NONE, 5), (APPLY_MAX_HP, 0x0102), (APPLY_NONE, 0)],
        );
        let protos = ItemProtos::from_rows(vec![costume.clone(), mount.clone(), sword.clone()]);
        assert_eq!(
            item_applies(&item(1, 41_001), &costume, &protos),
            [(APPLY_NONE, 5), (APPLY_MAX_HP, 0x0102), (APPLY_NONE, 0)]
        );
        assert_eq!(item_applies(&item(2, 71_224), &mount, &protos), []);
        assert_eq!(
            item_applies(&item(3, 10), &sword, &protos),
            [(APPLY_MAX_HP, 0x0102)]
        );
    }

    #[test]
    fn the_attributes_apply_except_on_four_event_items() {
        let ring = proto(71_135, ITEM_ARMOR, ARMOR_BODY);
        let other = proto(71_134, ITEM_ARMOR, ARMOR_BODY);
        let protos = ItemProtos::from_rows(vec![ring.clone(), other.clone()]);
        let with_attributes = |vnum| {
            let mut worn = item(1, vnum);
            worn.attributes[1] = ItemAttribute::new(APPLY_MAX_HP, 0x0203);
            worn.attributes[6] = ItemAttribute::new(APPLY_SKILL, -0x0102);
            worn
        };
        assert_eq!(
            item_applies(&with_attributes(71_134), &other, &protos),
            [(APPLY_MAX_HP, 0x0203), (APPLY_SKILL, -0x0102)]
        );
        for vnum in ATTRIBUTES_SKIPPED {
            let skipped = proto(vnum, ITEM_ARMOR, ARMOR_BODY);
            assert_eq!(item_applies(&with_attributes(vnum), &skipped, &protos), []);
        }
    }

    #[test]
    fn a_weapons_element_applies_its_attack_and_enchantment() {
        let sword = proto(10, ITEM_WEAPON, 0);
        let armour = proto(11_200, ITEM_ARMOR, ARMOR_BODY);
        let protos = ItemProtos::from_rows(vec![sword.clone(), armour.clone()]);
        let refined = |element, proto: &ItemProto| {
            let mut worn = item(1, proto.vnum);
            worn.refine_element = element;
            item_applies(&worn, proto, &protos)
        };
        assert_eq!(
            refined(301_234_000, &sword),
            [(APPLY_ATT_GRADE_BONUS, 34), (APPLY_ENCHANT_ELECT + 2, 12)]
        );
        assert_eq!(refined(600_100_000, &sword), [(APPLY_ENCHANT_ELECT + 5, 1)]);
        // A seventh element keeps its attack and grants no enchantment.
        assert_eq!(refined(701_234_000, &sword), [(APPLY_ATT_GRADE_BONUS, 34)]);
        // The largest element reads as element 42 with an attack of 67.
        assert_eq!(refined(u32::MAX, &sword), [(APPLY_ATT_GRADE_BONUS, 67)]);
        // No element, and an element on an armour, apply nothing.
        assert_eq!(refined(1_234_000, &sword), []);
        assert_eq!(refined(301_234_000, &armour), []);
        assert_eq!(enchant(0), None);
        assert_eq!(enchant(1), Some(APPLY_ENCHANT_ELECT));
        assert_eq!(enchant(6), Some(APPLY_ENCHANT_ELECT + 5));
        assert_eq!(enchant(7), None);
        assert_eq!(enchant(0x0100 + 1), None);
    }

    #[test]
    fn the_sash_scale_rounds_half_up_and_truncates_toward_zero() {
        for (value, pct, expected) in [
            (149, 1, 1),
            (150, 1, 2),
            (-149, 1, 0),
            (-150, 1, -1),
            (-151, 1, -1),
            (-250, 1, -2),
            (0x0203, 25, 129),
            (i64::from(i32::MAX), 100, i32::MAX),
            (i64::from(i32::MAX), 200, i32::MAX),
            (i64::from(i32::MIN), 200, i32::MIN),
        ] {
            assert_eq!(scale(value, pct), expected, "{value} * {pct}%");
        }
    }

    #[test]
    fn the_worn_items_are_read_from_the_wear_cells_in_order() {
        let armour = applying(
            valued(proto(11_210, ITEM_ARMOR, ARMOR_BODY), [0, 40, 0, 0, 0, 3]),
            [(APPLY_MAX_HP, 0x0102), (APPLY_NONE, 0), (APPLY_NONE, 0)],
        );
        let sword = applying(proto(10, ITEM_WEAPON, 0), [(APPLY_MAX_SP, 0x0304); 3]);
        let protos = ItemProtos::from_rows(vec![armour, sword]);
        let body = item(1, 11_210);
        let weapon = item(2, 10);
        let unknown = item(3, 55_555);
        let mut items = wearing(&[(WEAPON, &weapon), (BODY, &body), (HEAD, &unknown)]);
        // An item in the bag is not worn.
        items
            .set(ItemPos::new(INV, 3), &item(4, 10))
            .expect("a free bag cell");
        let equipment = Equipment::of(&items, &protos);
        let worn: Vec<(u16, u32)> = equipment
            .worn()
            .map(|(wear, worn)| (wear, worn.item.id))
            .collect();
        assert_eq!(
            worn,
            [(BODY, 1), (WEAPON, 2)],
            "an item with no proto is not worn"
        );
        let applied: Vec<(u32, Vec<(u8, i32)>)> = equipment
            .modify_points()
            .into_iter()
            .map(|(worn, applies)| (worn.item.id, applies))
            .collect();
        assert_eq!(
            applied,
            [
                (1, vec![(APPLY_MAX_HP, 0x0102)]),
                (2, vec![(APPLY_MAX_SP, 0x0304); 3])
            ]
        );
        assert_eq!(equipment.armour(), 46);
        assert!(equipment.wear(HEAD).is_none());
        assert!(equipment.wear(u16::MAX).is_none());
        let nothing = Equipment::default();
        assert!(nothing.modify_points().is_empty());
        assert_eq!(nothing.armour(), 0);
        assert!(nothing.set_bonus_applies().is_empty());
    }

    #[test]
    fn the_armour_counts_body_head_foot_and_shield_armours() {
        let armours = [
            (BODY, 11_210, ARMOR_BODY, [0, 0x0102, 0, 0, 0, 0x0003]),
            (HEAD, 12_210, ARMOR_HEAD, [0, 0x0010, 0, 0, 0, 0x0001]),
            (FOOTS, 15_010, ARMOR_FOOTS, [0, 0x0020, 0, 0, 0, 0]),
            (SHIELD, 13_010, ARMOR_SHIELD, [0, 0x0040, 0, 0, 0, 0x0002]),
            (WRIST, 14_010, ARMOR_WRIST, [0, 0x1000, 0, 0, 0, 0x1000]),
            (NECK, 16_010, ARMOR_NECK, [0, 0x1000, 0, 0, 0, 0x1000]),
        ];
        let mut rows: Vec<ItemProto> = armours
            .iter()
            .map(|(_, vnum, sub, values)| valued(proto(*vnum, ITEM_ARMOR, *sub), *values))
            .collect();
        rows.push(valued(
            proto(10, ITEM_WEAPON, 0),
            [0, 0x1000, 0, 0, 0, 0x1000],
        ));
        let protos = ItemProtos::from_rows(rows);
        let items: Vec<(u16, Item)> = armours
            .iter()
            .zip(1..)
            .map(|((wear, vnum, _, _), id)| (*wear, item(id, *vnum)))
            .chain([(WEAPON, item(99, 10))])
            .collect();
        let refs: Vec<(u16, &Item)> = items.iter().map(|(wear, item)| (*wear, item)).collect();
        let worn = wearing(&refs);
        let equipment = Equipment::of(&worn, &protos);
        assert_eq!(
            equipment.armour(),
            0x0102 + 6 + 0x0010 + 2 + 0x0020 + 0x0040 + 4
        );
    }

    #[test]
    fn the_parts_start_from_the_base_and_the_kept_parts() {
        let current = [9, 0x0405, 7, 0x0607, 0x0809, 11];
        assert_eq!(
            Equipment::default().parts(0x0203, current),
            [0x0203, 0x0405, 0, 0x0607, 0x0809, 0]
        );
    }

    #[test]
    fn a_worn_weapon_armour_and_costume_set_their_parts() {
        let rows = vec![
            proto(0x0001_0203, ITEM_WEAPON, 0),
            proto(11_210, ITEM_ARMOR, ARMOR_BODY),
            proto(12_210, ITEM_ARMOR, ARMOR_HEAD),
            proto(41_001, ITEM_COSTUME, COSTUME_BODY),
            proto(40_100, ITEM_COSTUME, COSTUME_WEAPON),
            proto(29_101, ITEM_PICK, 0),
            proto(27_400, ITEM_ROD, 0),
        ];
        let protos = ItemProtos::from_rows(rows);
        let parts_of = |worn: &[(u16, &Item)]| {
            let items = wearing(worn);
            Equipment::of(&items, &protos).parts(0x0203, [0; PARTS])
        };
        let main = EParts::Main as usize;
        let weapon_part = EParts::Weapon as usize;
        let sword = item(1, 0x0001_0203);
        // The part keeps the low 16 bits of the vnum.
        assert_eq!(parts_of(&[(WEAPON, &sword)])[weapon_part], 0x0203);
        let mut looked = sword.clone();
        looked.transmutation = 0x0304;
        assert_eq!(parts_of(&[(WEAPON, &looked)])[weapon_part], 0x0304);
        // A weapon anywhere but the weapon cell shows nothing.
        assert_eq!(parts_of(&[(UNIQUE1, &sword)])[weapon_part], 0);
        // A costume weapon hides the weapon, whichever is read first.
        let mut costume_weapon = item(2, 40_100);
        costume_weapon.transmutation = 0x0506;
        let parts = parts_of(&[(WEAPON, &sword), (COSTUME_WEAPON_CELL, &costume_weapon)]);
        assert_eq!(parts[weapon_part], 0x0506);
        // A pick shows its own vnum, never its look, and only from the weapon cell.
        let mut pick = item(3, 29_101);
        pick.transmutation = 0x0506;
        assert_eq!(parts_of(&[(WEAPON, &pick)])[weapon_part], 29_101);
        assert_eq!(parts_of(&[(UNIQUE1, &item(4, 27_400))])[weapon_part], 0);
        // A body armour sets the body from any cell; a head armour sets nothing.
        let mut body = item(5, 11_210);
        body.transmutation = 11_299;
        assert_eq!(parts_of(&[(BODY, &body)])[main], 11_299);
        assert_eq!(parts_of(&[(UNIQUE1, &body)])[main], 11_299);
        assert_eq!(
            parts_of(&[(HEAD, &item(6, 12_210))]),
            [0x0203, 0, 0, 0, 0, 0]
        );
        // A costume body hides the armour.
        let costume = item(7, 41_001);
        let parts = parts_of(&[(BODY, &body), (COSTUME_BODY_CELL, &costume)]);
        assert_eq!(parts[main], 41_001);
        // Legacy asks whether the costume cell holds an item, not whether it holds a costume, so
        // any item there hides the weapon or the armour and shows nothing itself.
        let parts = parts_of(&[
            (WEAPON, &sword),
            (COSTUME_WEAPON_CELL, &item(8, 0x0001_0203)),
        ]);
        assert_eq!(parts, [0x0203, 0, 0, 0, 0, 0]);
        let parts = parts_of(&[(BODY, &body), (COSTUME_BODY_CELL, &item(9, 12_210))]);
        assert_eq!(parts, [0x0203, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_hair_sash_and_aura_costume_set_their_parts() {
        let rows = vec![
            valued(
                proto(45_001, ITEM_COSTUME, COSTUME_HAIR),
                [0, 0, 0, 0x0001_0405, 0, 0],
            ),
            valued(
                proto(45_002, ITEM_COSTUME, COSTUME_HAIR),
                [0, 0, 0, 0x0506, 0, 0],
            ),
            valued(
                proto(45_003, ITEM_COSTUME, COSTUME_HAIR),
                [0, 0, 0, -2, 0, 0],
            ),
            proto(85_001, ITEM_COSTUME, COSTUME_SASH),
            proto(80_000, ITEM_COSTUME, COSTUME_SASH),
            proto(49_001, ITEM_COSTUME, COSTUME_AURA),
            proto(71_224, ITEM_COSTUME, COSTUME_MOUNT),
        ];
        let protos = ItemProtos::from_rows(rows);
        let parts_of = |worn: &[(u16, &Item)]| {
            let items = wearing(worn);
            Equipment::of(&items, &protos).parts(0, [0, 0, 0, 0x0102, 0x0304, 0])
        };
        let hair = EParts::Hair as usize;
        let sash = EParts::Sash as usize;
        let aura = EParts::Aura as usize;
        let mut hair_item = item(1, 45_001);
        assert_eq!(parts_of(&[(COSTUME_HAIR_CELL, &hair_item)])[hair], 0x0405);
        // A look takes the hair from the look's proto, or its own when that proto is missing.
        hair_item.transmutation = 45_002;
        assert_eq!(parts_of(&[(COSTUME_HAIR_CELL, &hair_item)])[hair], 0x0506);
        hair_item.transmutation = 99_999;
        assert_eq!(parts_of(&[(COSTUME_HAIR_CELL, &hair_item)])[hair], 0x0405);
        assert_eq!(
            parts_of(&[(COSTUME_HAIR_CELL, &item(2, 45_003))])[hair],
            0xfffe
        );
        // A sash shows its vnum past 85000, and 2000 more from an absorption of 19.
        let mut sash_item = socketed(item(3, 85_001), [18, 0, 0, 0, 0, 0]);
        assert_eq!(parts_of(&[(COSTUME_SASH_CELL, &sash_item)])[sash], 1);
        sash_item.sockets[0] = 19;
        assert_eq!(parts_of(&[(COSTUME_SASH_CELL, &sash_item)])[sash], 2_001);
        sash_item.transmutation = 85_010;
        assert_eq!(parts_of(&[(COSTUME_SASH_CELL, &sash_item)])[sash], 2_010);
        // 80000 - 85000 wraps as a DWORD, and the part keeps the low 16 bits.
        let low = item(4, 80_000);
        assert_eq!(parts_of(&[(COSTUME_SASH_CELL, &low)])[sash], 0xec78);
        // An aura shows only from the aura cell, and a mount shows nothing.
        let aura_item = item(5, 49_001);
        assert_eq!(parts_of(&[(COSTUME_AURA_CELL, &aura_item)])[aura], 49_001);
        assert_eq!(parts_of(&[(UNIQUE1, &aura_item)])[aura], 0);
        assert_eq!(
            parts_of(&[(EWearPositions::CostumeMount as u16, &item(6, 71_224))]),
            [0, 0, 0, 0x0102, 0x0304, 0]
        );
    }

    #[test]
    fn the_set_bonus_counts_the_pieces_in_their_cells() {
        let first = &NEW_SETS[0].slots;
        let second = &NEW_SETS[1].slots;
        let rows: Vec<ItemProto> = first
            .iter()
            .chain(second)
            .map(|(_, vnum)| proto(*vnum, ITEM_ARMOR, ARMOR_BODY))
            .collect();
        let protos = ItemProtos::from_rows(rows);
        let bonus_of = |worn: &[(u16, u32)]| {
            let items: Vec<(u16, Item)> = worn
                .iter()
                .zip(1..)
                .map(|((wear, vnum), id)| (*wear, item(id, *vnum)))
                .collect();
            let refs: Vec<(u16, &Item)> = items.iter().map(|(wear, item)| (*wear, item)).collect();
            let items = wearing(&refs);
            Equipment::of(&items, &protos).set_bonus_applies()
        };
        // Six pieces of the first set give four bonuses, and two of the second give one.
        let mut worn: Vec<(u16, u32)> = first[..6].to_vec();
        worn.extend_from_slice(&second[6..]);
        assert_eq!(
            bonus_of(&worn),
            [
                (APPLY_MAX_HP, 500),
                (APPLY_ATTBONUS_MONSTER, 3),
                (APPLY_ATTBONUS_BOSS, 3),
                (APPLY_ATTBONUS_BOSS, 3),
                (APPLY_MAX_HP, 3_000),
            ]
        );
        assert_eq!(bonus_of(first).len(), 5, "a full set gives all five");
        assert_eq!(bonus_of(&first[..1]), [], "one piece gives nothing");
        // A piece in another cell does not count.
        let misplaced = [(first[0].0, first[0].1), (UNIQUE1, first[1].1)];
        assert_eq!(bonus_of(&misplaced), []);
        assert!(is_set_item(19));
        assert!(is_set_item(17_109));
        assert!(!is_set_item(18));
        assert!(!is_set_item(0));
    }

    #[test]
    fn the_part_word_keeps_the_low_sixteen_bits() {
        assert_eq!(word(0x0001_0203), 0x0203);
        assert_eq!(word(0xfffe_0405), 0x0405);
        assert_eq!(word(0x0506), 0x0506);
    }
}
