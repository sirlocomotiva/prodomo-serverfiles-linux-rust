//! The toggle arm of `UseItemEx` (`case ITEM_TOGGLE`, `G/char_item.cpp:7149-7152`, and
//! `G/ItemUtils.cpp:58-196`).
//!
//! An active toggle deactivates: socket 3 goes 0, the item unlocks, and the follower
//! unsummons (`DeactivateToggleItem`, `:104-132`). An inactive one activates
//! (`ActivateToggleItem`, `:58-102`): past 1 the stack unstacks first — the count drops
//! by one and a fresh single item takes the first free base cell, refusing with
//! `Not enough space in your inventory` when there is none (`:61-76`) — then socket 3
//! goes 1, the item locks, and the follower summons. The group check is the caller's:
//! it reads the whole inventory, which the world step cannot see past its own cells. The
//! owner's proto reads group 0 (value 5) and no second toggle exists, so the check
//! always passes.
//!
//! The lock is the caller's too: it reads the summon map, which lives on the game
//! state. The world answers which toggle flipped and which way, with the socket write,
//! the unstack's count and set, and the store changes.

use gamedata::item_kind::{ITEM_TOGGLE, TOGGLE_SHAMAN};
use gamedata::item_proto::ItemProto;
use protocol::item_pos::ItemPos;

use super::equip::{use_checks, use_source, Gear};
use super::item_move::{
    ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, MoveRefused, MoveRules, Unported,
};
use super::items::CharacterItems;
use crate::item::{Item, ItemIds};
use common::item_slots::EWindows;

/// Socket 3 (`ITEM_SOCKET_TOGGLE_ACTIVE`, `C/item_length.h:51`): set, the toggle
/// is active; clear, it is not.
const ACTIVE_SOCKET: usize = 3;

/// Whether a prototype is the shaman toggle: `ITEM_TOGGLE`/`TOGGLE_SHAMAN` (69700).
#[must_use]
pub fn is_shaman_toggle(proto: &ItemProto) -> bool {
    proto.item_type == ITEM_TOGGLE && proto.sub_type == TOGGLE_SHAMAN
}

/// What a toggle's use came to, for the caller, which flips the summon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggled {
    /// The toggle activated: the follower summons for `id`.
    On {
        /// The activated item's id.
        id: u32,
    },
    /// The toggle deactivated: the follower of `id` goes.
    Off {
        /// The deactivated item's id.
        id: u32,
    },
}

/// Use the toggle at `at`: `UseItem`'s checks, then the toggle arm.
///
/// The group check is the caller's (see the module doc): `group_taken` is whether
/// another active toggle of the same group stands. Past it, the socket write lands with
/// the unstack (when the stack held more than one) and the caller flips the summon.
///
/// # Errors
///
/// As [`super::use_item`]'s checks, and [`MoveRefused::NotPorted`] without `gear` or
/// for an item that is no shaman toggle.
pub fn use_toggle_item(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    at: ItemPos,
    rules: &MoveRules,
    gear: Option<&mut Gear<'_>>,
    group_taken: bool,
) -> Result<(MoveDone, Toggled), MoveRefused> {
    let item = use_source(items, at)?;
    let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
    let proto = gear
        .protos
        .get(item.vnum)
        .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
    if !is_shaman_toggle(proto) {
        return Err(MoveRefused::NotPorted(Unported::Use(proto.item_type)));
    }
    use_checks(&item, proto, rules, gear.points)?;
    let mut done = MoveDone {
        kind: MoveKind::Used,
        records: Vec::new(),
        changes: Vec::new(),
    };
    // Set, the toggle deactivates; clear, it activates.
    if item.sockets[ACTIVE_SOCKET] != 0 {
        // `DeactivateToggleItem` (`G/ItemUtils.cpp:104-132`): socket 3 goes 0. The
        // owner's proto reads no wear timer, so no timer stops; the unlock and the
        // unsummon are the caller's.
        let sockets = items
            .set_socket(item.id, ACTIVE_SOCKET, 0)
            .map_err(|_| MoveRefused::Empty)?;
        let kept = items.item(item.id).ok_or(MoveRefused::Empty)?;
        done.records
            .push(MoveRecord::Item(ItemRecord::Update(kept.gc_item_update())));
        done.changes.push(ItemChange::Sockets {
            id: item.id,
            sockets,
        });
        return Ok((done, Toggled::Off { id: item.id }));
    }
    if group_taken {
        return Err(MoveRefused::ToggleGroupTaken);
    }
    // `ActivateToggleItem` (`G/ItemUtils.cpp:58-102`): past 1 the stack unstacks.
    if item.count > 1 {
        let Some(cell) = items.find_free_inventory_cell(rules.usable_cells, item.size) else {
            return Err(MoveRefused::NoRoomInInventory);
        };
        let ids = ids.ok_or(MoveRefused::NoItemIds)?;
        let id = ids.allocate().map_err(|_| MoveRefused::IdsExhausted)?;
        let left = item.count - 1;
        items
            .set_count(item.id, u32::from(left))
            .map_err(MoveRefused::Count)?;
        let kept = items
            .item(item.id)
            .ok_or(MoveRefused::Empty)?
            .gc_item_update();
        // `CreateItem(vnum)` and `AddToCharacter`: the fresh single item, with the
        // proto's shape, takes the first free base cell.
        let mut single = Item::new(id, item.vnum);
        single.flags = item.flags;
        single.anti_flags = item.anti_flags;
        single.size = item.size;
        let pos = ItemPos::new(EWindows::Inventory as u8, cell);
        items.set(pos, &single).map_err(MoveRefused::Storage)?;
        done.records
            .push(MoveRecord::Item(ItemRecord::Update(kept)));
        done.records.push(MoveRecord::Item(ItemRecord::Set(
            single.gc_item_set(pos, 0),
        )));
        done.changes.push(ItemChange::Count {
            id: item.id,
            count: left,
        });
        done.changes.push(ItemChange::Created(single));
    }
    // Socket 3 goes 1. The lock and the summon are the caller's.
    let sockets = items
        .set_socket(item.id, ACTIVE_SOCKET, 1)
        .map_err(|_| MoveRefused::Empty)?;
    let kept = items.item(item.id).ok_or(MoveRefused::Empty)?;
    done.records
        .push(MoveRecord::Item(ItemRecord::Update(kept.gc_item_update())));
    done.changes.push(ItemChange::Sockets {
        id: item.id,
        sockets,
    });
    Ok((done, Toggled::On { id: item.id }))
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use gamedata::item_kind::ITEM_TOGGLE;
    use gamedata::item_proto::ItemProto;
    use protocol::item_pos::ItemPos;

    use super::super::item_move::{MoveRecord, MoveRefused, MoveRules};
    use super::super::items::CharacterItems;
    use super::super::points::Points;
    use super::*;
    use crate::character::equip::Gear;
    use crate::character::FixedDraw;
    use crate::item::Item;

    const TOGGLE: u32 = 69_700;
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

    fn holding(
        count: u16,
        sockets: [i32; 6],
    ) -> (CharacterItems, gamedata::item_proto::ItemProtos) {
        let proto = ItemProto::for_category_rule(TOGGLE, ITEM_TOGGLE, TOGGLE_SHAMAN);
        let protos = gamedata::item_proto::ItemProtos::from_rows(vec![proto]);
        let mut item = Item::new(7, TOGGLE);
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

    fn flip(
        items: &mut CharacterItems,
        protos: &gamedata::item_proto::ItemProtos,
        group_taken: bool,
    ) -> Result<(MoveDone, Toggled), MoveRefused> {
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
        let mut ids =
            ItemIds::new(crate::item::ItemIdRange::new(100, 200, 100).expect("a valid range"));
        use_toggle_item(
            items,
            Some(&mut ids),
            AT,
            &RULES,
            Some(&mut gear),
            group_taken,
        )
    }

    /// The toggle's four answers (`G/ItemUtils.cpp:58-196`): a set socket clears with
    /// `Off`; a clear one sets with `On`; a stack of two unstacks (the count drops,
    /// a fresh single item lands at the first free cell); and a taken group refuses.
    #[test]
    fn the_toggle_flips_unstacks_and_refuses_its_group() {
        let (mut items, protos) = holding(1, [0, 0, 0, 1, 0, 0]);
        let (done, toggled) = flip(&mut items, &protos, false).expect("a set toggle flips off");
        assert_eq!(toggled, Toggled::Off { id: 7 });
        assert_eq!(items.item(7).expect("kept").sockets[3], 0);
        assert!(done
            .records
            .iter()
            .any(|record| matches!(record, MoveRecord::Item(_))));

        let (mut items, protos) = holding(1, [0, 0, 0, 0, 0, 0]);
        let (_, toggled) = flip(&mut items, &protos, false).expect("a clear toggle flips on");
        assert_eq!(toggled, Toggled::On { id: 7 });
        assert_eq!(items.item(7).expect("kept").sockets[3], 1);

        let (mut items, protos) = holding(2, [0, 0, 0, 0, 0, 0]);
        let (done, toggled) = flip(&mut items, &protos, false).expect("a stack unstacks");
        assert_eq!(toggled, Toggled::On { id: 7 });
        assert_eq!(items.item(7).expect("kept").count, 1);
        let free = ItemPos::new(EWindows::Inventory as u8, 0);
        let single = items.item_at(free).expect("the fresh single lands first");
        assert_eq!((single.vnum, single.count), (TOGGLE, 1));
        assert!(
            done.changes.iter().any(|change| matches!(
                change,
                ItemChange::Created(created) if created.vnum == TOGGLE
            )),
            "the unstack's fresh item"
        );

        let (mut items, protos) = holding(1, [0, 0, 0, 0, 0, 0]);
        assert_eq!(
            flip(&mut items, &protos, true).expect_err("a taken group refuses"),
            MoveRefused::ToggleGroupTaken
        );
    }
}
