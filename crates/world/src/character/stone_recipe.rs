//! The stone and the recipe arms of `UseItemEx` (`case USE_CREATE_STONE` and `case
//! USE_RECIPE`, `G/char_item.cpp:6866-6870`, `:6889-6943`).
//!
//! The stone scroll gives a metin of `number(28000, 28013)` and spends one of its
//! stack; the roll and the give run on the game state, which owns the dice and the
//! item ids. The recipe scroll brews value(0) out of two sources (value(1) of
//! value(2), value(3) of value(4)) and one 50901 bottle, and is itself never spent:
//! each source is the first stack of its vnum in the base inventory and then in the
//! custom banks (`FindSpecifyItem`, `G/char_item.cpp:8724-8742`), a wanted count of
//! 0 skips its stack whatever it holds, and the first source is already consumed
//! when the second's count is checked.

use gamedata::item_kind::{ITEM_USE, USE_CREATE_STONE, USE_RECIPE};
use gamedata::item_proto::ItemProtos;
use protocol::item_pos::ItemPos;

use super::equip::{use_checks, use_source, Trail};
use super::item_move::{done, ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, MoveRefused};
use super::items::CharacterItems;
use super::potion::use_one;
use super::quickslot::{chains_when_used_up, find_specify_item, QuickslotSync, SyncTo};
use crate::item::gc_item_clear;

/// Run the use checks on the stone or recipe scroll at `at`, changing nothing: the
/// stone's give and the recipe's brew run after their scroll passed, as legacy's
/// `UseItem` checks run before `UseItemEx`.
///
/// # Errors
///
/// As [`consume_use_scroll`].
pub fn check_use_scroll(
    items: &mut CharacterItems,
    at: ItemPos,
    rules: &super::item_move::MoveRules,
    gear: Option<&mut super::equip::Gear<'_>>,
) -> Result<MoveDone, MoveRefused> {
    let item = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(
        super::item_move::Unported::Equipment,
    ))?;
    let proto = gear
        .protos
        .get(item.vnum)
        .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    if proto.item_type != ITEM_USE
        || (proto.sub_type != USE_CREATE_STONE && proto.sub_type != USE_RECIPE)
    {
        return Err(MoveRefused::NotPorted(
            super::item_move::Unported::UseSubType(proto.sub_type),
        ));
    }
    use_checks(&item, proto, rules, gear.points)?;
    Ok(done(MoveKind::Used, Trail::default()))
}

/// Spend one of the stone scroll at `at` (`case USE_CREATE_STONE`,
/// `G/char_item.cpp:6866-6870`): the use checks, then one of the stack.
///
/// The roll and the give ran before, on the game state; the first-use stamp the
/// normal path lays before its step is skipped, as no stone or recipe scroll the
/// owner holds carries a first-use limit.
///
/// # Errors
///
/// As [`use_item`](super::equip::use_item): a missing or foreign source, no gear, an
/// unknown vnum or a use of no stone scroll, or a refused use.
pub fn consume_use_scroll(
    items: &mut CharacterItems,
    at: ItemPos,
    rules: &super::item_move::MoveRules,
    gear: Option<&mut super::equip::Gear<'_>>,
) -> Result<MoveDone, MoveRefused> {
    let item = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(
        super::item_move::Unported::Equipment,
    ))?;
    let proto = gear
        .protos
        .get(item.vnum)
        .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    if proto.item_type != ITEM_USE
        || (proto.sub_type != USE_CREATE_STONE && proto.sub_type != USE_RECIPE)
    {
        return Err(MoveRefused::NotPorted(
            super::item_move::Unported::UseSubType(proto.sub_type),
        ));
    }
    use_checks(&item, proto, rules, gear.points)?;
    let mut trail = Trail::default();
    use_one(items, &item, chains_when_used_up(proto), &mut trail)?;
    Ok(done(MoveKind::Used, trail))
}

/// Why [`take_recipe_source`] took nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipeTake {
    /// No stack of the vnum is held.
    Missing,
    /// The first stack holds less than wanted.
    Short,
}

/// Take `needed` of the first stack of `vnum` (`case USE_RECIPE`,
/// `G/char_item.cpp:6890-6918`), for one source or the bottle: a wanted count of 0
/// changes nothing and answers taken, whatever is held.
///
/// A depleted stack leaves with its quickslot sync; a kept one sends its new count.
///
/// # Errors
///
/// [`RecipeTake::Missing`] when no stack of `vnum` is held, [`RecipeTake::Short`]
/// when the first holds less than `needed`; both change nothing.
pub fn take_recipe_source(
    items: &mut CharacterItems,
    protos: &ItemProtos,
    vnum: u32,
    needed: u32,
) -> Result<MoveDone, RecipeTake> {
    let mut done = MoveDone {
        kind: MoveKind::Used,
        records: Vec::new(),
        changes: Vec::new(),
    };
    if needed == 0 {
        return Ok(done);
    }
    let Some(cell) = find_specify_item(items, vnum) else {
        return Err(RecipeTake::Missing);
    };
    let at = ItemPos::new(common::item_slots::EWindows::Inventory as u8, cell);
    let Some(item) = items.item_at(at).cloned() else {
        return Err(RecipeTake::Missing);
    };
    if u32::from(item.count) < needed {
        return Err(RecipeTake::Short);
    }
    if u32::from(item.count) == needed {
        let Ok(pos) = items.release(item.id) else {
            return Err(RecipeTake::Missing);
        };
        let clear = MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos)));
        if protos.get(vnum).is_some_and(chains_when_used_up) {
            done.records.push(clear);
            done.records.push(MoveRecord::QuickslotSync(QuickslotSync {
                from: cell,
                to: SyncTo::Chain(vnum),
            }));
        } else {
            done.records.push(MoveRecord::QuickslotSync(QuickslotSync {
                from: cell,
                to: SyncTo::Delete,
            }));
            done.records.push(clear);
        }
        done.changes.push(ItemChange::Destroyed { id: item.id });
        return Ok(done);
    }
    let keeps = item.count - u16::try_from(needed).unwrap_or(item.count);
    if items.set_count(item.id, u32::from(keeps)).is_err() {
        return Err(RecipeTake::Missing);
    }
    if let Some(kept) = items.item(item.id) {
        done.records
            .push(MoveRecord::Item(ItemRecord::Update(kept.gc_item_update())));
    }
    done.changes.push(ItemChange::Count {
        id: item.id,
        count: keeps,
    });
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::equip::Gear;
    use crate::character::points::PointsRow;
    use crate::character::Points;
    use crate::item::{Item, ITEM_FLAG_STACKABLE};
    use common::item_slots::EWindows;
    use gamedata::item_kind::ITEM_MATERIAL;
    use gamedata::item_proto::ItemProto;
    use gamedata::special_item_group::SpecialItemGroups;

    use crate::character::dice::FixedDraw;

    const INV: u8 = EWindows::Inventory as u8;
    const SCROLL: u32 = 27_999;
    const LEATHER: u32 = 50_701;

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(INV, cell)
    }

    fn protos() -> ItemProtos {
        let scroll = ItemProto::for_category_rule(SCROLL, ITEM_USE, USE_CREATE_STONE);
        let mut leather = ItemProto::for_category_rule(LEATHER, ITEM_MATERIAL, 0);
        leather.flags = ITEM_FLAG_STACKABLE;
        ItemProtos::from_rows(vec![scroll, leather])
    }

    fn rules() -> super::super::item_move::MoveRules {
        super::super::item_move::MoveRules {
            count_limit: 200,
            usable_cells: 90,
            belt_grade: None,
            questing: false,
            secured: false,
        }
    }

    /// A stone scroll spends one of its stack and keeps the rest; the last leaves.
    #[test]
    fn a_stone_scroll_spends_one_and_keeps_the_rest() {
        let protos = protos();
        let groups = SpecialItemGroups::default();
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
            premium: (0, 0),
            shop_unlocked: 0,
        });
        let mut dice = FixedDraw::new(50);
        let mut gear = Gear {
            points: &mut points,
            protos: &protos,
            groups: &groups,
            dice: &mut dice,
            recently_fought: false,
            first_use_now: None,
        };
        let mut items = CharacterItems::new();
        let mut scroll = Item::new(9, SCROLL);
        scroll.count = 2;
        items.set(inv(0), &scroll).expect("free");
        let done = consume_use_scroll(&mut items, inv(0), &rules(), Some(&mut gear))
            .expect("the scroll spends");
        assert_eq!(done.kind, MoveKind::Used);
        assert_eq!(items.item_at(inv(0)).expect("kept").count, 1);
        let done = consume_use_scroll(&mut items, inv(0), &rules(), Some(&mut gear))
            .expect("the last spends");
        assert_eq!(done.kind, MoveKind::Used);
        assert!(items.item_at(inv(0)).is_none(), "the last leaves");
    }

    /// The recipe's take answers its misses and spends its hits, one stack only.
    #[test]
    fn a_recipe_take_answers_its_misses_and_spends_its_hits() {
        let protos = protos();
        let mut items = CharacterItems::new();
        let mut leather = Item::new(10, LEATHER);
        leather.count = 3;
        leather.flags = ITEM_FLAG_STACKABLE;
        items.set(inv(1), &leather).expect("free");
        assert_eq!(
            take_recipe_source(&mut items, &protos, 50_702, 1),
            Err(RecipeTake::Missing)
        );
        assert_eq!(
            take_recipe_source(&mut items, &protos, LEATHER, 4),
            Err(RecipeTake::Short)
        );
        assert_eq!(items.item_at(inv(1)).expect("kept").count, 3);
        take_recipe_source(&mut items, &protos, LEATHER, 0).expect("zero takes nothing");
        assert_eq!(items.item_at(inv(1)).expect("kept").count, 3);
        take_recipe_source(&mut items, &protos, LEATHER, 2).expect("two taken");
        assert_eq!(items.item_at(inv(1)).expect("kept").count, 1);
        take_recipe_source(&mut items, &protos, LEATHER, 1).expect("the last taken");
        assert!(items.item_at(inv(1)).is_none(), "the stack leaves");
    }
}
