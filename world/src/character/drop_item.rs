//! Making a dropped item, as `CreateItem` makes one (`G/item_manager.cpp:160-455`).
//!
//! The drop path calls with `id == 0` and `bTryMagic = true`, and the limit rows pass no
//! rare percent (so `-1` selects the proto's `bAlterToMagicItemPct`) while the kill row
//! passes its `iRarePct`. What the other arms need is established by the drop vnums, not
//! by guessing at coverage:
//!
//! - No drop vnum is ELK, UNIQUE, DS, BLEND, an aura, a socketed vnum, QUEST, RING, or a
//!   skill book, and none sets MAKECOUNT (plan §2). Those arms are refused paths with the
//!   vnum named.
//! - Four armor vnums carry an addon and a magic percent; four costume vnums carry a
//!   `LIMIT_REAL_TIME` of 86400 s, one with an addon. Those arms run.
//! - No drop vnum is masked (no `ori_to_new_table.txt` new-vnum), but the mask runs: it
//!   is a table read, not a vnum property, so a later table edit cannot silently change
//!   what the client sees.
//!
//! The magic percent the limit rows pass is dropped with a comment: legacy's
//! `SetRarity` is display-only through the target-info preview (D8), and nothing reads
//! it before then.
//!
//! # The Gaussian
//!
//! `ApplyAddonTo`'s skill bonus draws `gauss_random(0, 5)` (`G/item_addon.cpp:28`), a
//! Marsaglia-polar generator over `uniform_random` legs (`G/utils.cpp:202-230`). Each
//! leg is `f64(draw) / f64(2^31) * 2 - 1`; two draws make one polar round, which redoes
//! while `s >= 1` or `s < f32::EPSILON` (legacy's `fabs(s) < FLT_EPSILON`, and `s` is
//! never negative); the cached second Gaussian is served next call. Legacy holds the
//! cache in a process static; here it lives on the dice holder ([`Gauss`]) and is
//! threaded through, so no state hides between calls.
//!
//! `G/` = `server/server/game/`.

use gamedata::item_kind::{
    ARMOR_BODY, ARMOR_EAR, ARMOR_FOOTS, ARMOR_GLOVE, ARMOR_HEAD, ARMOR_NECK, ARMOR_SHIELD,
    ARMOR_WRIST, COSTUME_AURA, COSTUME_BODY, COSTUME_HAIR, COSTUME_WEAPON, ITEM_ARMOR, ITEM_BLEND,
    ITEM_COSTUME, ITEM_DS, ITEM_ELK, ITEM_QUEST, ITEM_RING, ITEM_TALISMAN, ITEM_UNIQUE,
    ITEM_WEAPON, LIMIT_REAL_TIME, LIMIT_TIMER_BASED_ON_WEAR, WEAPON_ARROW,
};
use gamedata::item_proto::{ItemProto, ItemProtos};
use gamedata::records::ItemAttrRecord;
use protocol::gc_item_window::ItemAttribute;

use super::dice::{number, Dice};
use super::shop::SOCKETED_VNUMS;
use crate::item::{Item, ItemId, ItemIds, ITEM_FLAG_STACKABLE, SOCKETS};
use common::item_slots::ITEM_COUNT_LIMIT;

/// `ITEM_FLAG_MAKECOUNT` (`item_length.h:366`): the flag whose stackable items with a
/// count of 1 take the proto's `VALUE1` as their count.
const ITEM_FLAG_MAKECOUNT: u32 = 1 << 7;

/// `APPLY_SKILL_DAMAGE_BONUS`, the addon table's skill bonus (`length.h:567`).
const APPLY_SKILL_DAMAGE_BONUS: u8 = 71;

/// `APPLY_NORMAL_HIT_DAMAGE_BONUS`, the addon table's hit bonus (`length.h:568`).
const APPLY_NORMAL_HIT_DAMAGE_BONUS: u8 = 72;

/// `ITEM_ATTRIBUTE_MAX_LEVEL` (`item_length.h:35`): the five levels `PutAttribute` rolls
/// between.
const ATTRIBUTE_LEVELS: usize = 5;

/// `aiItemMagicAttributePercentHigh` (`G/constants.cpp:866-870`): the level walk of the
/// first magic attribute.
const MAGIC_HIGH: [i32; ATTRIBUTE_LEVELS] = [30, 40, 20, 8, 2];

/// `aiItemMagicAttributePercentLow` (`G/constants.cpp:872-876`): the level walk of the
/// second and third magic attributes.
const MAGIC_LOW: [i32; ATTRIBUTE_LEVELS] = [50, 40, 10, 0, 0];

/// `MAX_NORM_ATTR_NUM` (`item_length.h:32`): the magic and addon attributes stop here.
const NORM_ATTRS: usize = 5;

/// The non-armor sets `GetAttributeSetIndex` answers (`G/item_attribute.cpp:14-93`):
/// weapon 0, talisman 8, glove 9 (`length.h:794-805`, both glove switches defined).
const SET_WEAPON: usize = 0;
const SET_TALISMAN: usize = 8;
const SET_GLOVE: usize = 9;

/// The armor sets, in `ATTRIBUTE_SET_*` order: body 1 through ear 7.
const SET_BODY: usize = 1;
const SET_WRIST: usize = 2;
const SET_FOOTS: usize = 3;
const SET_NECK: usize = 4;
const SET_HEAD: usize = 5;
const SET_SHIELD: usize = 6;
const SET_EAR: usize = 7;

/// A path of `CreateItem` a dropped item would take that the Rewrite does not port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnportedDrop {
    /// `ITEM_ELK`: gold as an item, which is neither numbered nor counted.
    Elk,
    /// `ITEM_UNIQUE`: the remaining time in socket 2 and the expiry event.
    Unique,
    /// `ITEM_DS`: `DragonSoulItemInitialize` and the dragon soul inventory.
    DragonSoul,
    /// `ITEM_BLEND`: the blend table's bonus.
    Blend,
    /// `COSTUME_AURA`: the aura level in a socket.
    Aura,
    /// An auto potion or a skill book, whose socket `CreateItem` fills by vnum.
    Vnum(u32),
    /// `ITEM_QUEST`, `ITEM_RING`, `ITEM_UNIQUE` or `COSTUME_MOUNT`: the `SIGVnum` scans.
    SigVnum,
    /// `IsDragonSoul`: `DragonSoulItemInitialize`.
    Soul,
}

/// Why `make_drop_item` made nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropRefused {
    /// The vnum has no proto (`CreateItem` answers `NULL`).
    SoldOut,
    /// The item would take a path of `CreateItem` the Rewrite does not port.
    NotPorted(UnportedDrop),
    /// The item ids are used up.
    IdsExhausted,
}

impl core::fmt::Display for DropRefused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SoldOut => write!(f, "the item has no proto"),
            Self::NotPorted(path) => {
                write!(f, "making the item takes an unported path: {path:?}")
            }
            Self::IdsExhausted => write!(f, "the item ids are used up"),
        }
    }
}

impl std::error::Error for DropRefused {}

/// The cached second Gaussian of [`gauss_random`], which legacy keeps in a process
/// static (`G/utils.cpp:210-211`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Gauss {
    /// The cached `nextGaussian`, served by the next call.
    cached: Option<f32>,
}

impl Gauss {
    /// No cached Gaussian, as at process start.
    #[must_use]
    pub const fn new() -> Self {
        Self { cached: None }
    }
}

/// `gauss_random(avg, sigma)` (`G/utils.cpp:208-230`): the Marsaglia-polar draw over
/// `uniform_random(-1, 1)` legs, each `f64(draw) / f64(2^31) * 2 - 1`.
///
/// Legacy divides by `RAND_MAX + 1.f`: `random()` returns `0..2^31`, so the divisor is
/// `f64(2^31)` exactly. The polar round redoes while `s >= 1` or `s < f32::EPSILON`
/// (legacy's `fabs(s) < FLT_EPSILON`, and `s` is never negative); the multiplier is
/// `sqrt(-2 * ln(s) / s)` in `float` (`sqrtf`/`logf`), and the return widens to `f64`
/// only through `v1 * multiplier * sigma + avg` before narrowing back to `float`.
pub fn gauss_random(dice: &mut dyn Dice, cache: &mut Gauss, avg: f32, sigma: f32) -> f32 {
    if let Some(next) = cache.cached.take() {
        return next.mul_add(sigma, avg);
    }
    loop {
        let v1 = uniform_leg(dice.random31());
        let v2 = uniform_leg(dice.random31());
        let s = v1 * v1 + v2 * v2;
        if s >= 1.0 || s < f64::from(f32::EPSILON) {
            continue;
        }
        // The `as` casts are the legacy `sqrtf`/`logf` narrowing: the multiplier and the
        // legs are `float` from here on.
        #[allow(clippy::cast_possible_truncation)]
        let multiplier = (-2.0 * s.ln() / s).sqrt() as f32;
        #[allow(clippy::cast_possible_truncation)]
        let cached = v2 as f32 * multiplier;
        cache.cached = Some(cached);
        #[allow(clippy::cast_possible_truncation)]
        let out = (v1 as f32 * multiplier).mul_add(sigma, avg);
        return out;
    }
}

/// One `uniform_random(-1, 1)` leg (`G/utils.cpp:202-206`): `draw / 2^31 * 2 - 1`.
fn uniform_leg(draw: u32) -> f64 {
    f64::from(draw & 0x7fff_ffff) / f64::from(1_u32 << 31) * 2.0 - 1.0
}

/// `CItem::GetAttributeSetIndex` (`G/item_attribute.cpp:14-93`): the attribute set the
/// magic tables roll against, or `None` for the `-1` types (arrows, auras, anything
/// that is not a weapon, talisman, armor, or a costumed body, hair or weapon).
///
/// The costume arms follow the switches: `COSTUME_MOUNT` (`ENABLE_MOUNT_COSTUME_SYSTEM`)
/// and `COSTUME_PET` (`ENABLE_PET_COSTUME_SYSTEM`, both defined) break to `-1`;
/// `COSTUME_WEAPON` (`ENABLE_WEAPON_COSTUME_SYSTEM`, defined) rolls as a weapon; the
/// glove arm needs `ENABLE_GLOVE_SYSTEM` and `ENABLE_GLOVE_ITEM_ATTR` (both defined).
#[must_use]
pub fn attribute_set_index(item_type: i32, sub_type: i32) -> Option<usize> {
    if item_type == ITEM_WEAPON {
        if sub_type == WEAPON_ARROW {
            return None;
        }
        return Some(SET_WEAPON);
    }
    // `__AURA_SYSTEM__` is defined: a costume aura rolls no attributes.
    if item_type == ITEM_COSTUME && sub_type == COSTUME_AURA {
        return None;
    }
    if item_type == ITEM_TALISMAN {
        return Some(SET_TALISMAN);
    }
    if item_type == ITEM_ARMOR {
        return match sub_type {
            ARMOR_BODY => Some(SET_BODY),
            ARMOR_WRIST => Some(SET_WRIST),
            ARMOR_FOOTS => Some(SET_FOOTS),
            ARMOR_NECK => Some(SET_NECK),
            ARMOR_HEAD => Some(SET_HEAD),
            ARMOR_SHIELD => Some(SET_SHIELD),
            ARMOR_EAR => Some(SET_EAR),
            ARMOR_GLOVE => Some(SET_GLOVE),
            _ => None,
        };
    }
    if item_type == ITEM_COSTUME {
        // `ARMOR_BODY == COSTUME_BODY` and `ARMOR_HEAD == COSTUME_HAIR`, so a costume
        // body rolls as armor and costume hair as a helmet; mount and pet break out.
        return match sub_type {
            COSTUME_BODY => Some(SET_BODY),
            COSTUME_HAIR => Some(SET_HEAD),
            COSTUME_WEAPON => Some(SET_WEAPON),
            _ => None,
        };
    }
    None
}

/// The first path of `CreateItem` a dropped `vnum` would take that is not ported, in
/// the order `CreateItem` reaches them (`G/item_manager.cpp:160-455`).
#[must_use]
pub fn unported_drop(vnum: u32, proto: &ItemProto) -> Option<UnportedDrop> {
    match proto.item_type {
        ITEM_ELK => Some(UnportedDrop::Elk),
        ITEM_UNIQUE => Some(UnportedDrop::Unique),
        ITEM_DS => Some(UnportedDrop::Soul),
        ITEM_BLEND => Some(UnportedDrop::Blend),
        ITEM_COSTUME if proto.sub_type == COSTUME_AURA => Some(UnportedDrop::Aura),
        _ if SOCKETED_VNUMS.contains(&vnum) => Some(UnportedDrop::Vnum(vnum)),
        ITEM_QUEST | ITEM_RING => Some(UnportedDrop::SigVnum),
        _ => None,
    }
}

/// How many of `NORM_ATTRS` attributes `attributes` holds: the count of leading
/// nonzero types (`CItem::GetAttributeCount`, `G/item_attribute.cpp:287-298`).
fn attribute_count(attributes: &[ItemAttribute]) -> usize {
    attributes
        .iter()
        .take(NORM_ATTRS)
        .take_while(|attr| attr.b_type != 0)
        .count()
}

/// `CItem::HasAttr` (`G/item_attribute.cpp:95-107`): the proto's three applies or the
/// five normal attributes already hold `apply`.
fn has_attr(proto: &ItemProto, attributes: &[ItemAttribute], apply: u8) -> bool {
    proto.applies.iter().any(|slot| {
        let [kind, ..] = slot.kind.to_le_bytes();
        kind == apply
    }) || attributes
        .iter()
        .take(NORM_ATTRS)
        .any(|attr| attr.b_type == apply)
}

/// `CItem::AddAttribute` (`G/item_attribute.cpp:121-133`): `apply` with `value` at the
/// first free slot, unless the item already has it or the value is 0.
fn add_attribute(proto: &ItemProto, attributes: &mut [ItemAttribute], apply: u8, value: i16) {
    if has_attr(proto, attributes, apply) {
        return;
    }
    let at = attribute_count(attributes);
    if at < NORM_ATTRS && value != 0 {
        attributes[at] = ItemAttribute::new(apply, value);
    }
    // Past the fifth the legacy write overflows with a log line; creation never holds
    // five when the addon or the magic runs, so there is nothing to refuse.
}

/// `CItem::AddAttr` (`G/item_attribute.cpp:135-153`): the `level`-th value of the
/// attribute table's `apply` row, capped at the fifth level, unless the item has it.
fn add_attr(
    proto: &ItemProto,
    attributes: &mut [ItemAttribute],
    attrs: &[ItemAttrRecord],
    apply: u8,
    level: u8,
) {
    if has_attr(proto, attributes, apply) || level == 0 {
        return;
    }
    let at = attribute_count(attributes);
    if at >= NORM_ATTRS {
        return;
    }
    // `g_map_itemAttr[bApply]`: `std::map::operator[]` answers a zero row on a miss.
    let zero = ItemAttrRecord::default();
    let row = attrs
        .iter()
        .find(|row| row.apply_index == u32::from(apply))
        .unwrap_or(&zero);
    let capped = usize::from(level).min(row.values.len()) - 1;
    let value = row.values[capped];
    #[allow(clippy::cast_possible_truncation)]
    let value = value as i16;
    if value != 0 {
        attributes[at] = ItemAttribute::new(apply, value);
    }
}

/// `CItem::PutAttributeWithLevel` (`G/item_attribute.cpp:155-209`): one attribute of
/// `level` for `set`, drawn from the rows the set allows weighted by `prob`.
///
/// Every apply from 1 to `MAX_APPLY_NUM` whose row caps the set above 0 and the item
/// does not have joins the pool; the `number(1, total)` draw walks the pool in apply
/// order, subtracting each row's `prob` until the draw fits. The level is capped at
/// the row's cap for the set. A draw that fits nothing logs and adds nothing (legacy's
/// `!attr_idx` arm, which only a zero total reaches — `number` over an empty range).
fn put_attribute_with_level(
    dice: &mut dyn Dice,
    proto: &ItemProto,
    attributes: &mut [ItemAttribute],
    attrs: &[ItemAttrRecord],
    set: usize,
    level: u8,
) {
    let mut pool: Vec<(u8, u32, u8)> = Vec::new();
    let mut total: u32 = 0;
    for apply in 1..=u8::MAX {
        let zero = ItemAttrRecord::default();
        let row = attrs
            .iter()
            .find(|row| row.apply_index == u32::from(apply))
            .unwrap_or(&zero);
        let Some(&cap) = row.max_level_by_set.get(set) else {
            continue;
        };
        if cap != 0 && !has_attr(proto, attributes, apply) {
            pool.push((apply, row.prob, cap));
            total = total.saturating_add(row.prob);
        }
    }
    let mut pick =
        u32::try_from(number(dice, 1, i32::try_from(total).unwrap_or(i32::MAX))).unwrap_or(1);
    let mut chosen: Option<(u8, u8)> = None;
    for (apply, prob, cap) in pool {
        if pick <= prob {
            chosen = Some((apply, cap));
            break;
        }
        pick -= prob;
    }
    if let Some((apply, cap)) = chosen {
        add_attr(proto, attributes, attrs, apply, level.min(cap));
    }
}

/// `CItem::PutAttribute` (`G/item_attribute.cpp:212-225`): the level the table rolls,
/// then one attribute of that level.
fn put_attribute(
    dice: &mut dyn Dice,
    proto: &ItemProto,
    attributes: &mut [ItemAttribute],
    attrs: &[ItemAttrRecord],
    set: usize,
    table: &[i32; ATTRIBUTE_LEVELS],
) {
    let mut roll = number(dice, 1, 100);
    let mut level = ATTRIBUTE_LEVELS;
    for (at, &pct) in table.iter().enumerate() {
        if roll <= pct {
            level = at + 1;
            break;
        }
        roll -= pct;
    }
    #[allow(clippy::cast_possible_truncation)]
    put_attribute_with_level(dice, proto, attributes, attrs, set, level as u8);
}

/// `CItem::AlterToMagicItem` (`G/item.cpp:1807-1863`): the first attribute always, the
/// second and third on their rolls. Only weapons, armor and costumes roll; anything
/// else, and a type with no attribute set, keeps its attributes.
fn alter_to_magic_item(
    dice: &mut dyn Dice,
    proto: &ItemProto,
    attributes: &mut [ItemAttribute],
    attrs: &[ItemAttrRecord],
) {
    let Some(set) = attribute_set_index(proto.item_type, proto.sub_type) else {
        return;
    };
    let (second, third) = match proto.item_type {
        ITEM_WEAPON => (20, 5),
        ITEM_ARMOR | ITEM_COSTUME if proto.sub_type == ARMOR_BODY => (10, 2),
        // `__AURA_SYSTEM__` is defined but the aura arm is unreachable: the set index
        // above already answers `None` for a costume aura.
        ITEM_ARMOR | ITEM_COSTUME => (10, 1),
        _ => return,
    };
    put_attribute(dice, proto, attributes, attrs, set, &MAGIC_HIGH);
    if number(dice, 1, 100) <= second {
        put_attribute(dice, proto, attributes, attrs, set, &MAGIC_LOW);
    }
    if number(dice, 1, 100) <= third {
        put_attribute(dice, proto, attributes, attrs, set, &MAGIC_LOW);
    }
}

/// `CItemAddonManager::ApplyAddonTo` (`G/item_addon.cpp:16-52`): the skill and hit
/// bonus pair, added twice — the second pair is dead, because `HasAttr` keeps the
/// first (`G/item_attribute.cpp:95-107`).
///
/// `ENABLE_REWARD_SYSTEM` is defined (`prodomodefines.h:200`), but its arm reads the
/// item's owner, and a made item has none until it is laid, so no `DoReward` fires on
/// this path; the arm is a dead branch below, so a later owner-holding caller cannot
/// silently gain it.
fn apply_addon_to(
    dice: &mut dyn Dice,
    gauss: &mut Gauss,
    proto: &ItemProto,
    attributes: &mut [ItemAttribute],
) {
    #[allow(clippy::cast_possible_truncation)]
    let skill = (gauss_random(dice, gauss, 0.0, 5.0) + 0.5) as i32;
    let skill = skill.clamp(-30, 30);
    let normal = if skill.abs() <= 20 {
        -2 * skill + (number(dice, -8, 8) + number(dice, -8, 8)).abs() + number(dice, 1, 4)
    } else {
        -2 * skill + number(dice, 1, 5)
    };
    #[allow(clippy::cast_possible_truncation)]
    let (skill, normal) = (skill as i16, normal as i16);
    add_attribute(proto, attributes, APPLY_NORMAL_HIT_DAMAGE_BONUS, normal);
    add_attribute(proto, attributes, APPLY_SKILL_DAMAGE_BONUS, skill);
    // The dead second pair: `HasAttr` refuses both, so nothing changes.
    add_attribute(proto, attributes, APPLY_NORMAL_HIT_DAMAGE_BONUS, normal);
    add_attribute(proto, attributes, APPLY_SKILL_DAMAGE_BONUS, skill);
    // `ENABLE_REWARD_SYSTEM`: `GetOwner()` is null on a made item, so no reward.
    let _ = normal >= 45;
}

/// What `make_drop_item` needs beyond the protos and the dice.
pub struct DropMade<'a> {
    /// The boot-read `item_attr` rows, which the magic rolls against.
    pub attrs: &'a [ItemAttrRecord],
    /// The mask table: `new_vnum -> ori_vnum`.
    pub masks: &'a [(u32, u32)],
    /// `time(0)` at creation, in seconds: the base of a `LIMIT_REAL_TIME` socket.
    pub now_secs: i64,
    /// `g_bItemCountLimit`: the largest stack.
    pub count_limit: u16,
}

/// What the drop rows ask `make_drop_item` for: the row's count and rare percent.
#[derive(Debug, Clone, Copy)]
pub struct DropAsk {
    /// The row's count.
    pub count: i32,
    /// The row's rare percent, or `-1` for the proto's `bAlterToMagicItemPct`.
    pub rare_pct: i32,
}

/// `ITEM_MANAGER::CreateItem(vnum, count, 0, true, rare_pct)` (`G/item_manager.cpp:160-455`)
/// for a drop: the item a kill lays on the ground.
///
/// The count pins between 1 and `made.count_limit` for a stackable item (with the
/// `MAKECOUNT` arm, which no drop vnum sets) and is 1 otherwise. The sockets fill for
/// `bGainSocketPct` (capped at six); the limits stamp socket 0 (`LIMIT_REAL_TIME` from
/// `made.now_secs`, `LIMIT_TIMER_BASED_ON_WEAR` from its socket, value, or 10 h); the
/// addon and the magic (`rare_pct`, or the proto's `bAlterToMagicItemPct` on `-1`)
/// roll when the proto asks; the mask answers the shown vnum.
///
/// # Errors
///
/// [`DropRefused`], in the order legacy reaches them: no proto (which the drop callers
/// skip with `continue`), an unported arm, or no ids. Nothing changes on any of them.
pub fn make_drop_item(
    dice: &mut dyn Dice,
    gauss: &mut Gauss,
    ids: &mut ItemIds,
    protos: &ItemProtos,
    made: &DropMade<'_>,
    vnum: u32,
    ask: DropAsk,
) -> Result<Item, DropRefused> {
    if vnum == 0 {
        return Err(DropRefused::SoldOut);
    }
    let mask = made
        .masks
        .iter()
        .find(|&&(new, _)| new == vnum)
        .map_or(0, |&(_, ori)| ori);
    let proto = protos.get(vnum).ok_or(DropRefused::SoldOut)?;
    if let Some(path) = unported_drop(vnum, proto) {
        return Err(DropRefused::NotPorted(path));
    }
    let id: ItemId = ids.allocate().map_err(|_| DropRefused::IdsExhausted)?;
    let mut item = Item::new(id, vnum);
    if mask != 0 {
        item.vnum = mask;
    }
    let count = if proto.flags & ITEM_FLAG_STACKABLE == 0 {
        1
    } else {
        let mut count = ask.count.clamp(1, i32::from(made.count_limit));
        if count <= 1 && proto.flags & ITEM_FLAG_MAKECOUNT != 0 {
            count = proto.values[1].max(1);
        }
        u32::try_from(count).unwrap_or(1)
    };
    // The clamp above keeps the count within the limit the store enforces, so this
    // cannot refuse; a larger configured limit than the store's is refused honestly.
    item.set_count(count.min(u32::from(ITEM_COUNT_LIMIT)))
        .map_err(|_| DropRefused::SoldOut)?;
    item.size = {
        let [size, ..] = proto.size.to_le_bytes();
        size
    };
    item.flags = proto.flags;
    item.anti_flags = proto.anti_flags;
    // The gain-socket loop (`AlterToSocketItem`, `G/item.cpp:1795-1805`): `bGainSocketPct`
    // is a `BYTE`, so the low byte; past six the whole array fills.
    let [socket_pct, ..] = proto.gain_socket_pct.to_le_bytes();
    for socket in item
        .sockets
        .iter_mut()
        .take(usize::from(socket_pct).min(SOCKETS))
    {
        *socket = 1;
    }
    for limit in &proto.limits {
        if limit.kind == LIMIT_REAL_TIME {
            // `time(0) + value`, or a week when the value is 0 (`:287-288`); the ground
            // destroy sweep already covers a lying item, so the socket only marks it.
            let span = i64::from(limit.value).max(0);
            let span = if span == 0 { 7 * 24 * 60 * 60 } else { span };
            #[allow(clippy::cast_possible_truncation)]
            let at = made.now_secs.saturating_add(span) as i32;
            item.sockets[0] = at;
        } else if limit.kind == LIMIT_TIMER_BASED_ON_WEAR {
            // Not equipped and made fresh: its socket, or the value, or 10 h (`:306`).
            let mut span = i64::from(item.sockets[0]);
            if span == 0 {
                span = i64::from(limit.value);
            }
            if span == 0 {
                span = 10 * 60 * 60;
            }
            #[allow(clippy::cast_possible_truncation)]
            let span = span as i32;
            item.sockets[0] = span;
        }
    }
    // `SetRarity(iRarePct)`: dropped — display-only through the target-info preview
    // (D8), which nothing reads before then.
    let _ = ask.rare_pct;
    if proto.addon_type != 0 {
        apply_addon_to(dice, gauss, proto, &mut item.attributes);
    }
    let mut pct = ask.rare_pct;
    if pct == -1 {
        let [proto_pct, ..] = proto.alter_to_magic_item_pct.to_le_bytes();
        pct = i32::from(proto_pct);
    }
    if number(dice, 1, 100) <= pct {
        alter_to_magic_item(dice, proto, &mut item.attributes, made.attrs);
    }
    Ok(item)
}

#[cfg(test)]
mod tests {
    use gamedata::item_kind::{
        ARMOR_FOOTS, ARMOR_HEAD, ITEM_ARMOR, ITEM_TOTEM, ITEM_USE, LIMIT_LEVEL, USE_ABILITY_UP,
    };
    use gamedata::item_proto::{ItemProtos, ItemValue};

    use super::*;
    use crate::item::ItemIdRange;

    /// Draws from a script, in order; past the end repeats the last.
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

    fn ids() -> ItemIds {
        let range = ItemIdRange::new(1, 100, 1).unwrap();
        ItemIds::new(range)
    }

    fn no_ids() -> ItemIds {
        let range = ItemIdRange::new(1000, 1001, 1000).expect("one id");
        let mut ids = ItemIds::new(range);
        let _ = ids.allocate();
        ids
    }

    fn stackable(vnum: u32) -> ItemProto {
        let mut proto = ItemProto::for_category_rule(vnum, ITEM_USE, USE_ABILITY_UP);
        proto.flags = ITEM_FLAG_STACKABLE;
        proto
    }

    fn table(rows: Vec<ItemProto>) -> ItemProtos {
        ItemProtos::from_rows(rows)
    }

    fn made<'a>(attrs: &'a [ItemAttrRecord], masks: &'a [(u32, u32)]) -> DropMade<'a> {
        DropMade {
            attrs,
            masks,
            now_secs: 1_790_000_000,
            count_limit: 5000,
        }
    }

    #[test]
    fn plain_stackable_takes_the_row_count() {
        let protos = table(vec![stackable(27870)]);
        let mut dice = Script::new(vec![50]);
        let mut gauss = Gauss::new();
        // The magic gate draws once: 50 <= 0 never, so no attributes.
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[]),
            27870,
            DropAsk {
                count: 3,
                rare_pct: -1,
            },
        )
        .unwrap();
        assert_eq!(item.vnum, 27870);
        assert_eq!(item.count, 3);
        assert_eq!(item.sockets, [0; SOCKETS]);
        assert!(item.attributes.iter().all(|attr| attr.b_type == 0));
    }

    #[test]
    fn count_pins_between_1_and_the_limit() {
        let protos = table(vec![stackable(27870)]);
        let mut dice = Script::new(vec![50]);
        let mut gauss = Gauss::new();
        let low = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[]),
            27870,
            DropAsk {
                count: 0,
                rare_pct: -1,
            },
        )
        .unwrap();
        assert_eq!(low.count, 1);
        let high = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[]),
            27870,
            DropAsk {
                count: 99999,
                rare_pct: -1,
            },
        )
        .unwrap();
        assert_eq!(high.count, 5000);
    }

    #[test]
    fn non_stackable_takes_1() {
        let proto = ItemProto::for_category_rule(15370, ITEM_ARMOR, ARMOR_FOOTS);
        let protos = table(vec![proto]);
        let mut dice = Script::new(vec![50]);
        let mut gauss = Gauss::new();
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[]),
            15370,
            DropAsk {
                count: 7,
                rare_pct: -1,
            },
        )
        .unwrap();
        assert_eq!(item.count, 1);
    }

    #[test]
    fn no_proto_is_sold_out() {
        let protos = table(vec![]);
        let mut dice = Script::new(vec![1]);
        let mut gauss = Gauss::new();
        assert_eq!(
            make_drop_item(
                &mut dice,
                &mut gauss,
                &mut ids(),
                &protos,
                &made(&[], &[]),
                999,
                DropAsk {
                    count: 1,
                    rare_pct: -1,
                },
            ),
            Err(DropRefused::SoldOut)
        );
    }

    #[test]
    fn unported_arms_name_themselves() {
        for (vnum, item_type, sub_type, path) in [
            (1, ITEM_ELK, 0, UnportedDrop::Elk),
            (71001, ITEM_UNIQUE, 0, UnportedDrop::Unique),
            (50513, ITEM_DS, 0, UnportedDrop::Soul),
            (50821, ITEM_BLEND, 0, UnportedDrop::Blend),
            (72501, ITEM_COSTUME, COSTUME_AURA, UnportedDrop::Aura),
            (72723, ITEM_USE, USE_ABILITY_UP, UnportedDrop::Vnum(72723)),
            (50300, ITEM_USE, USE_ABILITY_UP, UnportedDrop::Vnum(50300)),
            (70037, ITEM_USE, USE_ABILITY_UP, UnportedDrop::Vnum(70037)),
            (30111, ITEM_QUEST, 0, UnportedDrop::SigVnum),
            (71135, ITEM_RING, 0, UnportedDrop::SigVnum),
        ] {
            let proto = ItemProto::for_category_rule(vnum, item_type, sub_type);
            assert_eq!(unported_drop(vnum, &proto), Some(path), "vnum {vnum}");
        }
    }

    #[test]
    fn no_ids_is_exhausted() {
        let protos = table(vec![stackable(27870)]);
        let mut dice = Script::new(vec![1]);
        let mut gauss = Gauss::new();
        assert_eq!(
            make_drop_item(
                &mut dice,
                &mut gauss,
                &mut no_ids(),
                &protos,
                &made(&[], &[]),
                27870,
                DropAsk {
                    count: 1,
                    rare_pct: -1,
                },
            ),
            Err(DropRefused::IdsExhausted)
        );
    }

    #[test]
    fn mask_answers_the_shown_vnum() {
        let protos = table(vec![stackable(39001)]);
        let mut dice = Script::new(vec![50]);
        let mut gauss = Gauss::new();
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[(39001, 25041)]),
            39001,
            DropAsk {
                count: 1,
                rare_pct: -1,
            },
        )
        .unwrap();
        assert_eq!(item.vnum, 25041);
    }

    #[test]
    fn gain_socket_pct_fills_that_many_capped_at_six() {
        let mut proto = stackable(27870);
        proto.gain_socket_pct = 2;
        let protos = table(vec![proto]);
        let mut dice = Script::new(vec![50]);
        let mut gauss = Gauss::new();
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[]),
            27870,
            DropAsk {
                count: 1,
                rare_pct: -1,
            },
        )
        .unwrap();
        assert_eq!(item.sockets, [1, 1, 0, 0, 0, 0]);
        let mut proto = stackable(27870);
        proto.gain_socket_pct = 200;
        let protos = table(vec![proto]);
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[]),
            27870,
            DropAsk {
                count: 1,
                rare_pct: -1,
            },
        )
        .unwrap();
        assert_eq!(item.sockets, [1; SOCKETS]);
    }

    #[test]
    fn real_time_stamps_now_plus_value() {
        let mut proto = ItemProto::for_category_rule(41004, ITEM_COSTUME, COSTUME_BODY);
        proto.limits[0] = ItemValue {
            kind: LIMIT_REAL_TIME,
            value: 86400,
        };
        let protos = table(vec![proto]);
        let mut dice = Script::new(vec![50]);
        let mut gauss = Gauss::new();
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[]),
            41004,
            DropAsk {
                count: 1,
                rare_pct: -1,
            },
        )
        .unwrap();
        assert_eq!(item.sockets[0], 1_790_000_000 + 86400);
    }

    #[test]
    fn real_time_zero_value_stamps_a_week() {
        let mut proto = ItemProto::for_category_rule(41004, ITEM_COSTUME, COSTUME_BODY);
        proto.limits[0] = ItemValue {
            kind: LIMIT_REAL_TIME,
            value: 0,
        };
        let protos = table(vec![proto]);
        let mut dice = Script::new(vec![50]);
        let mut gauss = Gauss::new();
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[]),
            41004,
            DropAsk {
                count: 1,
                rare_pct: -1,
            },
        )
        .unwrap();
        assert_eq!(item.sockets[0], 1_790_000_000 + 7 * 24 * 60 * 60);
    }

    #[test]
    fn wear_timer_falls_back_to_10h() {
        let mut proto = ItemProto::for_category_rule(41004, ITEM_COSTUME, COSTUME_BODY);
        proto.limits[0] = ItemValue {
            kind: LIMIT_TIMER_BASED_ON_WEAR,
            value: 0,
        };
        let protos = table(vec![proto]);
        let mut dice = Script::new(vec![50]);
        let mut gauss = Gauss::new();
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[]),
            41004,
            DropAsk {
                count: 1,
                rare_pct: -1,
            },
        )
        .unwrap();
        assert_eq!(item.sockets[0], 10 * 60 * 60);
    }

    #[test]
    fn gauss_legs_pass_and_cache() {
        // Legs (0.5, 0.0): s = 0.25, which passes; the cached leg is 0 * mult = 0.
        let mut dice = Script::new(vec![(1 << 30) | (1 << 29), 1 << 30]);
        let mut gauss = Gauss::new();
        let first = gauss_random(&mut dice, &mut gauss, 0.0, 5.0);
        assert!((first - 8.325_546).abs() < 0.001, "first {first}");
        // The cache serves next: 0 * sigma + avg.
        let second = gauss_random(&mut dice, &mut gauss, 3.0, 5.0);
        assert!(
            (second - 3.0).abs() < f32::EPSILON,
            "cached leg serves exactly: {second}"
        );
    }

    #[test]
    fn gauss_redoes_a_failed_round() {
        // Legs (0, 0): s = 0 < eps, redo; then (0.5, 0.0): s = 0.25, pass.
        let mut dice = Script::new(vec![1 << 30, 1 << 30, (1 << 30) | (1 << 29), 1 << 30]);
        let mut gauss = Gauss::new();
        let first = gauss_random(&mut dice, &mut gauss, 0.0, 5.0);
        assert!((first - 8.325_546).abs() < 0.001, "first {first}");
        assert_eq!(dice.at, 4);
    }

    #[test]
    fn addon_adds_the_pair_once() {
        let mut proto = ItemProto::for_category_rule(15370, ITEM_ARMOR, ARMOR_FOOTS);
        proto.addon_type = 1;
        proto.limits[0] = ItemValue {
            kind: LIMIT_LEVEL,
            value: 65,
        };
        let protos = table(vec![proto]);
        // Gauss legs (0.5, 0.0) -> skill 8; number(-8,8): draws 0,0 -> -8,-8;
        // number(1,4): draw 0 -> 1. Normal = -16 + 16 + 1 = 1.
        let mut dice = Script::new(vec![(1 << 30) | (1 << 29), 1 << 30, 0, 0, 0, 99]);
        let mut gauss = Gauss::new();
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&[], &[]),
            15370,
            DropAsk {
                count: 1,
                rare_pct: -1,
            },
        )
        .unwrap();
        // The pair lands once; the dead second pair changes nothing.
        assert_eq!(item.attributes[0], ItemAttribute::new(72, 1));
        assert_eq!(item.attributes[1], ItemAttribute::new(71, 8));
        assert!(item.attributes[2..].iter().all(|attr| attr.b_type == 0));
        // The magic gate drew last (99 <= 0 never): no magic attributes.
        assert_eq!(dice.at, 6);
    }

    #[test]
    fn minus_one_takes_the_proto_pct() {
        let mut proto = ItemProto::for_category_rule(15370, ITEM_ARMOR, ARMOR_FOOTS);
        proto.alter_to_magic_item_pct = 100;
        let protos = table(vec![proto]);
        // The gate passes (1 <= 100); the level roll then the avail walk draw.
        let mut dice = Script::new(vec![0, 0, 0]);
        let mut gauss = Gauss::new();
        let attrs = vec![ItemAttrRecord {
            apply: [0; 33],
            apply_index: 5,
            prob: 10,
            values: [6, 6, 8, 10, 12],
            max_level_by_set: [0, 0, 0, 1, 0, 0, 0, 0, 0, 0],
        }];
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&attrs, &[]),
            15370,
            DropAsk {
                count: 1,
                rare_pct: -1,
            },
        )
        .unwrap();
        // Level roll draw 0 -> 1 <= 30: level 1; avail walk draw 0 -> 1 <= 10: STR.
        // Foots cap 1, so level 1: values[0] = 6.
        assert_eq!(item.attributes[0], ItemAttribute::new(5, 6));
    }

    #[test]
    fn magic_gate_uses_the_row_pct() {
        let proto = ItemProto::for_category_rule(15370, ITEM_ARMOR, ARMOR_FOOTS);
        let protos = table(vec![proto]);
        // Gate draw 41 -> 42 <= 50 passes; level + avail + second/third gates.
        let mut dice = Script::new(vec![41, 0, 0, 99, 99]);
        let mut gauss = Gauss::new();
        let attrs = vec![ItemAttrRecord {
            apply: [0; 33],
            apply_index: 5,
            prob: 10,
            values: [6, 6, 8, 10, 12],
            max_level_by_set: [0, 0, 0, 1, 0, 0, 0, 0, 0, 0],
        }];
        let item = make_drop_item(
            &mut dice,
            &mut gauss,
            &mut ids(),
            &protos,
            &made(&attrs, &[]),
            15370,
            DropAsk {
                count: 1,
                rare_pct: 50,
            },
        )
        .unwrap();
        assert_eq!(item.attributes[0], ItemAttribute::new(5, 6));
        // Second gate draw 99 -> 100 <= 10 never; third never: one attribute.
        assert!(item.attributes[1..].iter().all(|attr| attr.b_type == 0));
    }

    #[test]
    fn attribute_sets_follow_legacy() {
        use gamedata::item_kind::{
            ARMOR_EAR, ARMOR_NECK, ARMOR_SHIELD, ARMOR_WRIST, COSTUME_HAIR, COSTUME_MOUNT,
            COSTUME_PET, COSTUME_WEAPON, ITEM_TALISMAN, ITEM_WEAPON, WEAPON_ARROW, WEAPON_BOW,
        };
        assert_eq!(attribute_set_index(ITEM_WEAPON, WEAPON_BOW), Some(0));
        assert_eq!(attribute_set_index(ITEM_WEAPON, WEAPON_ARROW), None);
        assert_eq!(attribute_set_index(ITEM_TALISMAN, 0), Some(8));
        assert_eq!(attribute_set_index(ITEM_ARMOR, ARMOR_BODY), Some(1));
        assert_eq!(attribute_set_index(ITEM_ARMOR, ARMOR_WRIST), Some(2));
        assert_eq!(attribute_set_index(ITEM_ARMOR, ARMOR_FOOTS), Some(3));
        assert_eq!(attribute_set_index(ITEM_ARMOR, ARMOR_NECK), Some(4));
        assert_eq!(attribute_set_index(ITEM_ARMOR, ARMOR_HEAD), Some(5));
        assert_eq!(attribute_set_index(ITEM_ARMOR, ARMOR_SHIELD), Some(6));
        assert_eq!(attribute_set_index(ITEM_ARMOR, ARMOR_EAR), Some(7));
        assert_eq!(attribute_set_index(ITEM_ARMOR, ARMOR_GLOVE), Some(9));
        assert_eq!(attribute_set_index(ITEM_COSTUME, COSTUME_BODY), Some(1));
        assert_eq!(attribute_set_index(ITEM_COSTUME, COSTUME_HAIR), Some(5));
        assert_eq!(attribute_set_index(ITEM_COSTUME, COSTUME_WEAPON), Some(0));
        assert_eq!(attribute_set_index(ITEM_COSTUME, COSTUME_MOUNT), None);
        assert_eq!(attribute_set_index(ITEM_COSTUME, COSTUME_PET), None);
        assert_eq!(attribute_set_index(ITEM_COSTUME, COSTUME_AURA), None);
        assert_eq!(attribute_set_index(ITEM_TOTEM, 0), None);
    }
}
