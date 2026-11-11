//! The recall portals of `UseItemEx` (`case USE_TALISMAN`, `G/char_item.cpp:5862-5958`).
//!
//! A town portal (value(0) 1) with socket 0 unset marks when its owner stands on no
//! living dungeon's index (`GiveRecallItem`, `:2067-2122`), then recalls the owner to
//! its empire's town (`GetRecallPositionByEmpire`, `G/sectree_manager.cpp:493-532`);
//! with socket 0 set it recalls to the marked spot (`ProcessRecallItem`, `:2124-2182`).
//! A memory portal (value(0) 2) with socket 0 unset marks, unless its owner stands on
//! a living dungeon's index (`[LS;391]`); with socket 0 set it recalls to the marked
//! spot. Any other value(0) answers nothing, as legacy's arm falls through.
//!
//! The arm's own gates run first, in legacy's order: maps 200 and 113 refuse with
//! `[LS;388]`, an arena map with `[LS;1205]`, a consent-duel fight with the
//! `no_potions_on_pvp` flag up and a vnum off the allow list with `[LS;1205]`
//! (`ENABLE_NEWSTUFF`, `G/arena.cpp:1105-1124`), and under 30% of maximum hit points
//! with `[LS;1152]` (`CalculateConsume`, `:2667-2698`). The warp-event gate (`:5888`)
//! never refuses: legacy never arms `m_pkWarpEvent` (its only writes are the `NULL`
//! construction and the destructor's cancel), so `IsWarping` is always false.
//!
//! The recall pays `CalculateConsume`'s life: 30% of maximum hit points, trimmed so at
//! least 10% stays. The town warp sends the user to the recall point, and the marking
//! stamps the user's spot onto the portal's sockets 0 and 1. The caller pays, warps,
//! and sends the lines, which go before the records, as legacy's `ChatPacket` goes
//! before the `UpdatePacket`s.

use common::item_slots::EWindows;
use gamedata::item_kind::{ITEM_USE, USE_TALISMAN};
use gamedata::item_proto::ItemProto;
use protocol::item_pos::ItemPos;

use super::equip::{use_checks, use_source, Gear, Trail};
use super::item_move::{
    done, ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, MoveRefused, MoveRules, Unported,
};
use super::items::CharacterItems;
use super::quickslot::chains_when_used_up;
use crate::item::ItemIds;

/// Value(0) 1: the town portal, which marks outside a dungeon and recalls to town.
pub const TOWN_PORTAL: i32 = 1;
/// Value(0) 2: the memory portal, which marks outside a dungeon and recalls to the mark.
pub const MEMORY_PORTAL: i32 = 2;

/// Maps 200 and 113, where no portal runs (`:5869-5873`).
pub const BARRED_MAPS: [i32; 2] = [200, 113];

/// The share of maximum hit points a recall costs (`WARP_NEED_LIFE_PERCENT`, `:2669`).
pub const WARP_NEED_LIFE_PERCENT: i32 = 30;
/// The share that must stay (`WARP_MIN_LIFE_PERCENT`, `:2670`).
pub const WARP_MIN_LIFE_PERCENT: i32 = 10;

/// The vnums the duel gate lets through (`IsAllowedPotionOnPVP`, `G/arena.cpp:1105-1124`).
pub const PVP_ALLOWED: [u32; 10] = [
    27004, 27005, 27006, 39040, 39041, 39042, 72727, 72728, 72729, 72730,
];

/// What the world reads around the user: its map, its empire, whether it is an arena
/// map, whether a living dungeon holds it, whether it fights a consent duel with the
/// `no_potions_on_pvp` flag up, its hit points and maximum, and its spot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalGround {
    /// The user's map.
    pub map: i32,
    /// The user's empire.
    pub empire: u8,
    /// Whether the map is an arena map.
    pub arena_map: bool,
    /// Whether a living dungeon holds the user's map.
    pub in_dungeon: bool,
    /// Whether the user fights a consent duel with `no_potions_on_pvp` up.
    pub pvp_gated: bool,
    /// The user's hit points and maximum.
    pub hp: (i32, i32),
    /// Where the user stands.
    pub spot: (i32, i32),
}

/// What a portal's use came to, for the caller, which pays, warps, and sends the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Portaled {
    /// Map 200 or 113: `[LS;388]`, and nothing changed.
    Barred,
    /// An arena map, or a gated duel off the allow list: `[LS;1205]`, and nothing changed.
    NoPortal,
    /// Under 30% of maximum hit points: `[LS;1152]`, and nothing changed.
    TooWeak,
    /// The user's empire is not the standing map's (`GiveRecallItem`'s first gate,
    /// `:2086-2091`): `[LS;1119]`, and nothing changed.
    WrongMap,
    /// No free base-inventory cell for the split mark (`:2117-2121`): `[LS;1130]`, and
    /// nothing changed.
    NoRoom,
    /// A memory mark inside a dungeon: `[LS;391]` with the item's name, and nothing changed.
    DungeonMark,
    /// The town portal's mark landed (outside a dungeon); the recall pays `consume` life
    /// and warps to town. `marked` is whether sockets were stamped.
    TownWarp {
        /// The life the recall pays.
        consume: i32,
        /// Whether the mark landed.
        marked: bool,
    },
    /// The portal recalls to its mark; the recall pays `consume` life and spends one.
    Recall {
        /// The life the recall pays.
        consume: i32,
    },
    /// The memory mark landed; nothing is paid and nobody warps.
    Marked,
    /// A value(0) that is neither portal: nothing is said, and nothing changed.
    Plain,
}

/// `GiveRecallItem`'s and `ProcessRecallItem`'s empire of a map index
/// (`G/char_item.cpp:2070-2084`, `:2130-2144`): 1 under 20, 2 under 40, 3 under 60,
/// none on 66 and 216, 0 under 10000, and -1 past it. `-1` passes every empire.
#[must_use]
pub const fn empire_of_map_index(index: i32) -> i32 {
    if index == 66 || index == 216 {
        return -1;
    }
    if index < 20 {
        return 1;
    }
    if index < 40 {
        return 2;
    }
    if index < 60 {
        return 3;
    }
    if index < 10_000 {
        return 0;
    }
    -1
}

/// Whether a prototype is a recall portal.
#[must_use]
pub fn is_portal(proto: &ItemProto) -> bool {
    proto.item_type == ITEM_USE && proto.sub_type == USE_TALISMAN
}

/// `CalculateConsume` (`G/char_item.cpp:2667-2698`): the life a recall pays, or `None`
/// under 30% of the maximum.
#[must_use]
pub fn calculate_consume(hp: i32, max_hp: i32) -> Option<i32> {
    let need = max_hp.saturating_mul(WARP_NEED_LIFE_PERCENT) / 100;
    if hp < need {
        return None;
    }
    let min = max_hp.saturating_mul(WARP_MIN_LIFE_PERCENT) / 100;
    let mut consume = need;
    if hp.saturating_sub(need) < min {
        consume = hp.saturating_sub(min);
    }
    Some(consume.max(0))
}

/// Use the portal at `at`: `UseItem`'s checks, then the portal arm. `ground` is what the
/// world reads around the user. `ids` mints the split mark's id, as `CreateItem` does.
///
/// # Errors
///
/// As [`super::use_item`]'s checks, [`MoveRefused::NoItemIds`] with no allocator when a
/// stacked portal marks, and [`MoveRefused::NotPorted`] without `gear` or for an item
/// that is no portal.
pub fn use_portal(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    at: ItemPos,
    rules: &MoveRules,
    gear: Option<&mut Gear<'_>>,
    ground: PortalGround,
) -> Result<(MoveDone, Portaled), MoveRefused> {
    let item = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
    let proto = gear
        .protos
        .get(item.vnum)
        .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    if !is_portal(proto) {
        return Err(MoveRefused::NotPorted(Unported::UseSubType(proto.sub_type)));
    }
    use_checks(&item, proto, rules, gear.points)?;
    let unchanged = |portaled| Ok((done(MoveKind::Used, Trail::default()), portaled));
    if BARRED_MAPS.contains(&ground.map) {
        return unchanged(Portaled::Barred);
    }
    if ground.arena_map || (ground.pvp_gated && !PVP_ALLOWED.contains(&item.vnum)) {
        return unchanged(Portaled::NoPortal);
    }
    let Some(consume) = calculate_consume(ground.hp.0, ground.hp.1) else {
        return unchanged(Portaled::TooWeak);
    };
    let portal = proto.values[0];
    if portal != TOWN_PORTAL && portal != MEMORY_PORTAL {
        return unchanged(Portaled::Plain);
    }
    if item.sockets[0] != 0 {
        let mut trail = Trail::default();
        super::potion::use_one(items, &item, chains_when_used_up(proto), &mut trail)?;
        return Ok((done(MoveKind::Used, trail), Portaled::Recall { consume }));
    }
    if portal == MEMORY_PORTAL && ground.in_dungeon {
        return unchanged(Portaled::DungeonMark);
    }
    if portal == TOWN_PORTAL && ground.in_dungeon {
        return unchanged(Portaled::TownWarp {
            consume,
            marked: false,
        });
    }
    // `GiveRecallItem` (`G/char_item.cpp:2067-2122`): the standing map's empire gate,
    // then the stamp. `-1` and 0 pass every empire (`if (iEmpireByMapIndex && ...)`).
    let wanted = empire_of_map_index(ground.map);
    if wanted > 0 && i32::from(ground.empire) != wanted {
        return unchanged(Portaled::WrongMap);
    }
    let mut trail = Trail::default();
    let stamped = stamp_mark(
        items,
        ids,
        &item,
        ground.spot,
        rules.usable_cells,
        &mut trail,
    )?;
    if !stamped {
        return unchanged(Portaled::NoRoom);
    }
    if portal == MEMORY_PORTAL {
        return Ok((done(MoveKind::Used, trail), Portaled::Marked));
    }
    Ok((
        done(MoveKind::Used, trail),
        Portaled::TownWarp {
            consume,
            marked: true,
        },
    ))
}

/// `GiveRecallItem`'s stamp (`G/char_item.cpp:2095-2121`): a lone portal takes the spot
/// on its own sockets 0 and 1; a stacked one splits a marked single into the first
/// free base-inventory cell and loses one. Answers whether the stamp landed; with no
/// free cell nothing changes and the caller sends `[LS;1130]`.
fn stamp_mark(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    item: &crate::item::Item,
    spot: (i32, i32),
    usable_cells: u16,
    trail: &mut Trail,
) -> Result<bool, MoveRefused> {
    if item.count <= 1 {
        let first = super::specify_item::set_item_socket(items, item.id, (0, spot.0));
        let second = super::specify_item::set_item_socket(items, item.id, (1, spot.1));
        trail.records.extend(first.records);
        trail.changes.extend(first.changes);
        trail.records.extend(second.records);
        trail.changes.extend(second.changes);
        return Ok(true);
    }
    let Some(cell) = items.find_free_inventory_cell(usable_cells, item.size) else {
        return Ok(false);
    };
    let ids = ids.ok_or(MoveRefused::NoItemIds)?;
    let id = ids.allocate().map_err(|_| MoveRefused::IdsExhausted)?;
    let mut marked = crate::item::Item::new(id, item.vnum);
    marked.size = item.size;
    marked.sockets[0] = spot.0;
    marked.sockets[1] = spot.1;
    let to = ItemPos::new(EWindows::Inventory as u8, cell);
    items.set(to, &marked).map_err(MoveRefused::Storage)?;
    let left = item.count.saturating_sub(1);
    if let Err(reason) = items.set_count(item.id, u32::from(left)) {
        let _ = items.release(id);
        return Err(MoveRefused::Count(reason));
    }
    let placed = items.item(id).cloned().ok_or(MoveRefused::Empty)?;
    let kept = items.item(item.id).cloned().ok_or(MoveRefused::Empty)?;
    trail
        .records
        .push(MoveRecord::Item(ItemRecord::Update(kept.gc_item_update())));
    trail
        .records
        .push(MoveRecord::Item(ItemRecord::Set(marked.gc_item_set(to, 0))));
    trail.changes.push(ItemChange::Count {
        id: item.id,
        count: left,
    });
    trail.changes.push(ItemChange::Created(placed));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use gamedata::item_kind::ITEM_USE;
    use gamedata::item_proto::{ItemProto, ItemProtos};
    use protocol::item_pos::ItemPos;

    use super::super::item_move::MoveRules;
    use super::super::items::CharacterItems;
    use super::super::points::Points;
    use super::*;
    use crate::character::equip::Gear;
    use crate::character::FixedDraw;
    use crate::item::{Item, ItemIdRange};

    const AT: ItemPos = ItemPos {
        window_type: EWindows::Inventory as u8,
        cell: 3,
    };

    const RULES: MoveRules = MoveRules {
        count_limit: 200,
        usable_cells: 90,
        belt_grade: None,
        questing: false,
        secured: false,
    };

    fn holding(portal: i32, count: u16, sockets: [i32; 6]) -> (CharacterItems, ItemProtos) {
        let mut proto = ItemProto::for_category_rule(22_000, ITEM_USE, USE_TALISMAN);
        proto.values[0] = portal;
        let protos = ItemProtos::from_rows(vec![proto]);
        let mut item = Item::new(7, 22_000);
        item.count = count;
        item.sockets = sockets;
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
            biologist: [0; 3],
            secured: (0, 999_999),
            premium: (0, 0),
            shop_unlocked: 0,
        })
    }

    fn ground(map: i32, hp: (i32, i32)) -> PortalGround {
        PortalGround {
            map,
            empire: 1,
            arena_map: false,
            in_dungeon: false,
            pvp_gated: false,
            hp,
            spot: (100, 200),
        }
    }

    fn run(
        items: &mut CharacterItems,
        protos: &ItemProtos,
        ground: PortalGround,
    ) -> (MoveDone, Portaled) {
        let mut points = warrior();
        let mut dice = FixedDraw::new(0);
        let groups = gamedata::special_item_group::SpecialItemGroups::default();
        let mut gear = Gear {
            points: &mut points,
            protos,
            groups: &groups,
            dice: &mut dice,
            recently_fought: false,
            first_use_now: None,
        };
        let range = ItemIdRange::new(100, 200, 100).expect("the range holds");
        let mut ids = ItemIds::new(range);
        use_portal(items, Some(&mut ids), AT, &RULES, Some(&mut gear), ground)
            .expect("the checks pass")
    }

    /// The consume is 30% of the maximum, trimmed so 10% stays, and `None` under 30%.
    #[test]
    fn the_recall_pays_thirty_percent_and_keeps_ten() {
        assert_eq!(calculate_consume(1000, 1000), Some(300));
        assert_eq!(calculate_consume(350, 1000), Some(250));
        assert_eq!(calculate_consume(299, 1000), None);
        assert_eq!(calculate_consume(1000, 0), Some(0));
    }

    /// Each ground answers its own outcome: the barred map, the arena, the weak, the
    /// wrong empire, and the full pack; a lone town portal marks and warps, a stacked
    /// one splits the mark off, a marked one recalls and spends, and a memory portal
    /// marks alone.
    #[test]
    fn each_portal_ground_answers_its_own_outcome() {
        let (mut items, protos) = holding(TOWN_PORTAL, 1, [0; 6]);
        let (_, got) = run(&mut items, &protos, ground(113, (1000, 1000)));
        assert_eq!(got, Portaled::Barred);
        let (mut items, protos) = holding(TOWN_PORTAL, 1, [0; 6]);
        let mut arena = ground(1, (1000, 1000));
        arena.arena_map = true;
        let (_, got) = run(&mut items, &protos, arena);
        assert_eq!(got, Portaled::NoPortal);
        let (mut items, protos) = holding(TOWN_PORTAL, 1, [0; 6]);
        let mut gated = ground(1, (1000, 1000));
        gated.pvp_gated = true;
        let (_, got) = run(&mut items, &protos, gated);
        assert_eq!(got, Portaled::NoPortal);
        let (mut items, protos) = holding(TOWN_PORTAL, 1, [0; 6]);
        let (_, got) = run(&mut items, &protos, ground(1, (299, 1000)));
        assert_eq!(got, Portaled::TooWeak);
        let (mut items, protos) = holding(TOWN_PORTAL, 1, [0; 6]);
        let mut foreign = ground(21, (1000, 1000));
        foreign.empire = 1;
        let (_, got) = run(&mut items, &protos, foreign);
        assert_eq!(got, Portaled::WrongMap);
        // A lone town portal marks its own sockets and warps.
        let (mut items, protos) = holding(TOWN_PORTAL, 1, [0; 6]);
        let (done, got) = run(&mut items, &protos, ground(1, (1000, 1000)));
        assert_eq!(
            got,
            Portaled::TownWarp {
                consume: 300,
                marked: true
            }
        );
        let marked = items.item_at(AT).expect("the portal stays");
        assert_eq!([marked.sockets[0], marked.sockets[1]], [100, 200]);
        assert_eq!(done.changes.len(), 2, "both socket writes ride");
        // A stacked town portal splits the mark into the first free cell.
        let (mut items, protos) = holding(TOWN_PORTAL, 2, [0; 6]);
        let (done, got) = run(&mut items, &protos, ground(1, (1000, 1000)));
        assert_eq!(
            got,
            Portaled::TownWarp {
                consume: 300,
                marked: true
            }
        );
        assert_eq!(items.item_at(AT).expect("the stack stays").count, 1);
        let split = items
            .item_at(ItemPos::new(EWindows::Inventory as u8, 0))
            .expect("the mark splits into cell 0");
        assert_eq!([split.sockets[0], split.sockets[1]], [100, 200]);
        assert_eq!(split.count, 1);
        assert!(done.records.len() == 2, "the update and the set ride");
        // A marked portal recalls and spends one.
        let (mut items, protos) = holding(TOWN_PORTAL, 2, [500, 600, 0, 0, 0, 0]);
        let (_, got) = run(&mut items, &protos, ground(1, (1000, 1000)));
        assert_eq!(got, Portaled::Recall { consume: 300 });
        assert_eq!(items.item_at(AT).expect("one stays").count, 1);
        // A memory portal marks and stays.
        let (mut items, protos) = holding(MEMORY_PORTAL, 1, [0; 6]);
        let (_, got) = run(&mut items, &protos, ground(1, (1000, 1000)));
        assert_eq!(got, Portaled::Marked);
        let marked = items.item_at(AT).expect("the portal stays");
        assert_eq!([marked.sockets[0], marked.sockets[1]], [100, 200]);
        // A memory mark inside a dungeon refuses.
        let (mut items, protos) = holding(MEMORY_PORTAL, 1, [0; 6]);
        let mut dungeon = ground(61, (1000, 1000));
        dungeon.in_dungeon = true;
        let (_, got) = run(&mut items, &protos, dungeon);
        assert_eq!(got, Portaled::DungeonMark);
        // A town portal inside a dungeon warps without marking.
        let (mut items, protos) = holding(TOWN_PORTAL, 1, [0; 6]);
        let mut dungeon = ground(61, (1000, 1000));
        dungeon.in_dungeon = true;
        let (_, got) = run(&mut items, &protos, dungeon);
        assert_eq!(
            got,
            Portaled::TownWarp {
                consume: 300,
                marked: false
            }
        );
        // A value(0) that is neither portal answers nothing.
        let (mut items, protos) = holding(9, 1, [0; 6]);
        let (_, got) = run(&mut items, &protos, ground(1, (1000, 1000)));
        assert_eq!(got, Portaled::Plain);
    }

    /// The empire ladder matches legacy's, including the 66/216 pass and the 10000 fold.
    #[test]
    fn the_map_ladder_matches_legacy() {
        for (index, wanted) in [
            (0, 1),
            (19, 1),
            (20, 2),
            (39, 2),
            (40, 3),
            (59, 3),
            (60, 0),
            (66, -1),
            (61, 0),
            (216, -1),
            (9999, 0),
            (10_000, -1),
            (301, 0),
        ] {
            assert_eq!(empire_of_map_index(index), wanted, "map {index}");
        }
    }
}
