//! The campfire arm of `UseItemEx` (`case ITEM_CAMPFIRE`, `G/char_item.cpp:2891-2947`).
//!
//! The item's checks run first (an `ITEM_USE` refusal, and no socket 0, among them), then
//! the arm's own: on map 113 nothing happens but `Forbidden`; within 60 s of the last
//! fire nothing happens but `Once at 60 seconds you can use the fire` (the
//! `__FIX_CAMPFIRE__` block, `:2894-2909`); past a missing sectree or a water cell the
//! lines are `[LS;1217]` and `[LS;1228]`. Else one fire is used up and the campfire
//! stands at the point 100 units ahead.

use gamedata::item_kind::ITEM_CAMPFIRE;
use gamedata::item_proto::ItemProto;
use protocol::item_pos::ItemPos;

use super::equip::{use_checks, use_source, Gear, Trail};
use super::item_move::{done, MoveDone, MoveKind, MoveRefused, MoveRules, Unported};
use super::items::CharacterItems;
use super::quickslot::chains_when_used_up;

/// `__FIX_CAMPFIRE__SEC` (`server/server/common/prodomodefines.h:105`).
pub const CAMPFIRE_COOLDOWN_SECS: u32 = 60;
/// Map 113, where no fire is lit (`:2894-2898`).
pub const CAMPFIRE_BARRED_MAP: i32 = 113;

/// What the world reads around the user: its map, its `kamp.ates` flag (0 for none),
/// `get_global_time()`, and the point 100 ahead with what the sectree says of it (`None`
/// for no tree, `Some((at, water))` else).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CampfireGround {
    /// The user's map.
    pub map: i32,
    /// Its `kamp.ates` flag, 0 for none.
    pub last: u32,
    /// `get_global_time()`.
    pub now: u32,
    /// The point 100 ahead, or `None` for no tree.
    pub ground: Option<((i32, i32), bool)>,
}

/// What a campfire's use came to, for the caller, which sends the line first and stands
/// the fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fired {
    /// Map 113: `Forbidden`, and nothing changed.
    Barred,
    /// Within 60 s of the last fire: the wait line, and nothing changed.
    Cooling,
    /// No sectree holds the point ahead: `[LS;1217]`, and nothing changed.
    NoTree,
    /// The point ahead is water: `[LS;1228]`, and nothing changed.
    Water,
    /// One fire went up and one was used: the campfire stands at the point 100 ahead.
    Lit,
}

/// Whether a prototype is a campfire.
#[must_use]
pub fn is_campfire(proto: &ItemProto) -> bool {
    proto.item_type == ITEM_CAMPFIRE
}

/// Use the campfire at `at`: `UseItem`'s checks, then the campfire arm.
///
/// # Errors
///
/// As [`super::use_item`]'s checks, and [`MoveRefused::NotPorted`] without `gear` or for
/// an item that is no campfire.
pub fn use_campfire(
    items: &mut CharacterItems,
    at: ItemPos,
    rules: &MoveRules,
    gear: Option<&mut Gear<'_>>,
    around: CampfireGround,
) -> Result<(MoveDone, Fired), MoveRefused> {
    let item = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
    let proto = gear
        .protos
        .get(item.vnum)
        .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    if !is_campfire(proto) {
        return Err(MoveRefused::NotPorted(Unported::Use(proto.item_type)));
    }
    use_checks(&item, proto, rules, gear.points)?;
    let unchanged = |fired| Ok((done(MoveKind::Used, Trail::default()), fired));
    if around.map == CAMPFIRE_BARRED_MAP {
        return unchanged(Fired::Barred);
    }
    if around.now.wrapping_sub(around.last) < CAMPFIRE_COOLDOWN_SECS {
        return unchanged(Fired::Cooling);
    }
    let Some((_, water)) = around.ground else {
        return unchanged(Fired::NoTree);
    };
    if water {
        return unchanged(Fired::Water);
    }
    let mut trail = Trail::default();
    super::potion::use_one(items, &item, chains_when_used_up(proto), &mut trail)?;
    Ok((done(MoveKind::Used, trail), Fired::Lit))
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use gamedata::item_kind::ITEM_CAMPFIRE;
    use gamedata::item_proto::ItemProto;
    use protocol::item_pos::ItemPos;

    use super::super::item_move::{MoveRecord, MoveRules};
    use super::super::items::CharacterItems;
    use super::super::points::Points;
    use super::*;
    use crate::character::equip::Gear;
    use crate::character::FixedDraw;
    use crate::item::Item;

    const AT: ItemPos = ItemPos {
        window_type: EWindows::Inventory as u8,
        cell: 3,
    };

    const RULES: MoveRules = MoveRules {
        count_limit: 200,
        usable_cells: 90,
        belt_grade: None,
        questing: false,
    };

    fn holding(count: u16) -> (CharacterItems, gamedata::item_proto::ItemProtos) {
        let proto = ItemProto::for_category_rule(27_600, ITEM_CAMPFIRE, 0);
        let protos = gamedata::item_proto::ItemProtos::from_rows(vec![proto]);
        let mut item = Item::new(7, 27_600);
        item.count = count;
        let mut items = CharacterItems::new();
        items.set(AT, &item).expect("the fixture places");
        (items, protos)
    }

    fn warrior() -> Points {
        Points::load(&super::super::points::PointsRow {
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
        })
    }

    fn ground(map: i32, last: u32, now: u32, at: Option<((i32, i32), bool)>) -> CampfireGround {
        CampfireGround {
            map,
            last,
            now,
            ground: at,
        }
    }

    /// Each ground answers its own outcome and only the lit one spends: the map 113 bar,
    /// the 60 s cool (and its lapse at exactly 60), the missing tree, the water, and the
    /// lit fire, which spends one of two (`G/char_item.cpp:2891-2947`,
    /// `server/server/common/prodomodefines.h:104-105`).
    #[test]
    fn each_campfire_ground_answers_its_own_outcome() {
        for (map, last, now, at, fired) in [
            (113, 0, 1_000, Some(((0, 0), false)), Fired::Barred),
            (41, 950, 1_000, Some(((0, 0), false)), Fired::Cooling),
            (41, 0, 1_000, None, Fired::NoTree),
            (41, 0, 1_000, Some(((0, 0), true)), Fired::Water),
            (41, 940, 1_000, Some(((0, 0), false)), Fired::Lit),
            (41, 0, 1_000, Some(((0, 0), false)), Fired::Lit),
        ] {
            let (mut items, protos) = holding(2);
            let mut points = warrior();
            let mut dice = FixedDraw::new(0);
            let mut gear = Gear {
                points: &mut points,
                protos: &protos,
                dice: &mut dice,
                recently_fought: false,
            };
            let (done, got) = use_campfire(
                &mut items,
                AT,
                &RULES,
                Some(&mut gear),
                ground(map, last, now, at),
            )
            .expect("the checks pass");
            assert_eq!(got, fired, "map {map} last {last}");
            let left = items.item_at(AT).map_or(0, |item| item.count);
            assert_eq!(left, if fired == Fired::Lit { 1 } else { 2 });
            assert!(done
                .records
                .iter()
                .all(|record| !matches!(record, MoveRecord::Effect(_))));
        }
    }
}
