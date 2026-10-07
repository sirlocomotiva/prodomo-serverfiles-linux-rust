//! Dropping an item to the ground and picking one up: `CHARACTER::DropItem`
//! (`G/char_item.cpp:7444`) and `CHARACTER::PickupItem` (`:7972`).
//!
//! These change one character's storage and answer with the records and row changes, as a
//! move does. Where the item lies, how long it lies there and who is told are the caller's:
//! the ground is not a character's. A drop deletes the slots that name the dropped cell,
//! as a [`QuickslotSync`] the caller runs (ledger 220).
//!
//! Not ported, and each is the caller's or a later ledger's: gold on the ground, the drop
//! time limit (the descriptor keeps it), ownership (a player's drop has no owner, so anyone
//! may pick it up), the party share, the quest events, the lock check, and the item log. While a
//! script waits for the character's client it drops nothing and picks up no quest item. A drop
//! of an item offered in a trade is refused silently (`IsExchanging`, `:7473`); a pickup may
//! still join an offered stack, as legacy's does.

use common::item_slots::{
    EWindows, CUSTOM_INVENTORY_SLOT_END, CUSTOM_INVENTORY_SLOT_START, INVENTORY_MAX_NUM,
};
use gamedata::item_custom_category::{is_custom_category, CATEGORY_NUM};
use gamedata::item_kind::ITEM_QUEST;
use gamedata::item_proto::ItemProtos;
use protocol::item_pos::ItemPos;

use super::dice::Dice;
use super::equip::roll_sash;
use super::inventory::{is_equip_position, is_valid_item_position};
use super::item_move::{
    is_flat_window, GroundRecord, ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord,
    MoveRefused, MoveRules, Unported,
};
use super::items::CharacterItems;
use super::quickslot::{QuickslotSync, SyncTo};
use crate::item::{
    gc_item_clear, Item, ItemIds, ITEM_ANTIFLAG_DROP, ITEM_ANTIFLAG_GIVE, ITEM_ANTIFLAG_STACK,
    ITEM_FLAG_STACKABLE,
};

/// `[LS;443]`: the item was dropped (`G/char_item.cpp:7536`).
pub const DROPPED_NOTICE: &str = "[LS;443]";

/// An item lying on the ground.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundItem {
    /// The VID the clients know it by.
    pub vid: u32,
    /// The item, whose `pos` no longer means anything.
    pub item: Item,
    /// Map x.
    pub x: i32,
    /// Map y.
    pub y: i32,
    /// `m_dwLastOwnerPID`: the store id of the character that last held it, or 0 for an item
    /// nobody has held. A pick-up by anyone else highlights the cell.
    pub last_owner: u32,
}

impl GroundItem {
    /// The `GC_ITEM_GROUND_ADD` a client that sees the item is sent.
    #[must_use]
    pub const fn add_record(&self) -> GroundRecord {
        GroundRecord::Add {
            vid: self.vid,
            vnum: self.item.vnum,
            x: self.x,
            y: self.y,
        }
    }
}

/// Where a drop puts the item, and who drops it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropAt {
    /// The VID the ground item takes.
    pub vid: u32,
    /// The dropper's x.
    pub x: i32,
    /// The dropper's y.
    pub y: i32,
    /// The dropper's store id.
    pub owner_id: u32,
    /// Whether a script of the dropper waits for its client (`quest::PC::IsRunning`).
    pub questing: bool,
    /// Whether a sectree holds the point, so `AddToGround` can lay the item there
    /// (`G/item.cpp:569-574`).
    pub lands: bool,
}

/// `CHARACTER::DropItem`: the item at `at`, or `count` of its stack, goes to the ground.
///
/// A `count` of 0 or more than the stack drops the whole item, which leaves the storage and
/// its row is deleted: legacy's `AddToGround` saves an item with no owner, and that save is a
/// `GD_ITEM_DESTROY` (`G/item_manager.cpp`). Otherwise the stack keeps the rest and a new
/// item with a fresh id, the same vnum and the stack's sockets goes to the ground, with no
/// row. Either way the records are the cell's, then the ground add, then [`DROPPED_NOTICE`].
///
/// # Errors
///
/// [`MoveRefused::InvalidSource`], [`MoveRefused::Empty`], [`MoveRefused::Exchanging`],
/// [`MoveRefused::DroppedWhileQuesting`], [`MoveRefused::Undroppable`],
/// [`MoveRefused::NoItemIds`] or [`MoveRefused::IdsExhausted`] for a part-stack,
/// [`MoveRefused::NotPorted`] for a window other than the inventory or a worn item, and
/// [`MoveRefused::NoSectree`] when the item would not land. Nothing changes on any of them.
pub fn drop_item(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    at: ItemPos,
    count: u16,
    to: DropAt,
) -> Result<(MoveDone, GroundItem), MoveRefused> {
    if !is_valid_item_position(at) {
        return Err(MoveRefused::InvalidSource);
    }
    if !is_flat_window(at.window_type) {
        return Err(MoveRefused::NotPorted(Unported::SourceWindow(
            at.window_type,
        )));
    }
    let item = items.item_at(at).cloned().ok_or(MoveRefused::Empty)?;
    if items.is_exchanging(item.id) {
        return Err(MoveRefused::Exchanging);
    }
    // `G/char_item.cpp:7479`, silently.
    if to.questing {
        return Err(MoveRefused::DroppedWhileQuesting);
    }
    if item.anti_flags & (ITEM_ANTIFLAG_DROP | ITEM_ANTIFLAG_GIVE) != 0 {
        return Err(MoveRefused::Undroppable);
    }
    if is_equip_position(at) {
        return Err(MoveRefused::NotPorted(Unported::Equipment));
    }
    // Legacy takes the item from its cell before `AddToGround` fails, and the item is lost,
    // with no line (`G/char_item.cpp:7507-7541`). The Rewrite refuses it after every check
    // legacy makes, so each line legacy sends is still sent (a Defect not reproduced).
    if !to.lands {
        return Err(MoveRefused::NoSectree);
    }
    let whole = count == 0 || count >= item.count;
    let (record, change, dropped, last_owner) = if whole {
        let pos = items.release(item.id).map_err(MoveRefused::Storage)?;
        let record = ItemRecord::Set(gc_item_clear(pos));
        (
            record,
            ItemChange::Destroyed { id: item.id },
            item,
            to.owner_id,
        )
    } else {
        let ids = ids.ok_or(MoveRefused::NoItemIds)?;
        let id = ids.allocate().map_err(|_| MoveRefused::IdsExhausted)?;
        let left = item.count - count;
        items
            .set_count(item.id, u32::from(left))
            .map_err(MoveRefused::Count)?;
        let kept = items
            .item(item.id)
            .ok_or(MoveRefused::Empty)?
            .gc_item_update();
        // `CreateItem` and `FN_copy_item_socket`: a new item, owned by nobody yet.
        let mut piece = Item::new(id, item.vnum);
        piece.count = count;
        piece.flags = item.flags;
        piece.anti_flags = item.anti_flags;
        piece.size = item.size;
        piece.sockets = item.sockets;
        let change = ItemChange::Count {
            id: item.id,
            count: left,
        };
        (ItemRecord::Update(kept), change, piece, 0)
    };
    let ground = GroundItem {
        vid: to.vid,
        item: dropped,
        x: to.x,
        y: to.y,
        last_owner,
    };
    // `SyncQuickslot(QUICKSLOT_TYPE_ITEM, Cell.cell, 255)` runs before the drop, so the
    // slots are deleted even when part of the stack stays.
    let sync = QuickslotSync {
        from: at.cell,
        to: SyncTo::Delete,
    };
    let done = MoveDone {
        kind: MoveKind::Dropped,
        records: vec![
            MoveRecord::QuickslotSync(sync),
            MoveRecord::Item(record),
            MoveRecord::Ground(ground.add_record()),
            MoveRecord::Notice(DROPPED_NOTICE),
        ],
        changes: vec![change],
    };
    Ok((done, ground))
}

/// Who picks an item up, and what they may hold.
pub struct Picker<'a> {
    /// The picker's store id.
    pub owner_id: u32,
    /// The picker's stack limit and unlocked cells.
    pub rules: &'a MoveRules,
    /// The prototypes, for the banks and the sash.
    pub protos: &'a ItemProtos,
    /// The draw a sash's absorption is rolled from.
    pub dice: &'a mut dyn Dice,
}

impl core::fmt::Debug for Picker<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Picker")
            .field("owner_id", &self.owner_id)
            .finish_non_exhaustive()
    }
}

/// The inventory cells `PickupItem` merges into, in its order: the base inventory, then the
/// custom banks (`G/char_item.cpp:8011-8018`).
fn merge_cells() -> impl Iterator<Item = u16> {
    (0..INVENTORY_MAX_NUM).chain(CUSTOM_INVENTORY_SLOT_START..CUSTOM_INVENTORY_SLOT_END)
}

/// `CHARACTER::PickupItem` for an item the picker may take.
///
/// A stackable item first tops up every stack of its vnum with the same sockets, in cell
/// order, up to the stack limit. Taken whole that way, it is gone from the ground: the stacks
/// are updated, the name is sent, and then the ground item goes. Otherwise it goes whole to
/// the first free cell of its banks, then of the unlocked base inventory, and is gone from the
/// ground before it is set. `ground.item.count` is what is left on the ground, which is only
/// less than before when the answer is [`MoveKind::Declined`].
///
/// # Errors
///
/// [`MoveRefused::UnknownVnum`] for an item with no prototype,
/// [`MoveRefused::PickedUpWhileQuesting`] for an `ITEM_QUEST` while a script of the picker
/// waits for its client (`@fixme150`, `G/char_item.cpp:7984-7993`), and
/// [`MoveRefused::NoRoomToPickUp`] when nothing merged and no cell is free. Nothing changes on
/// either. When something merged and no cell is free the answer is [`MoveKind::Declined`]:
/// the merges stand, with the notice last.
pub fn pickup_item(
    items: &mut CharacterItems,
    ground: &mut GroundItem,
    picker: &mut Picker<'_>,
) -> Result<MoveDone, MoveRefused> {
    let vnum = ground.item.vnum;
    let proto = picker
        .protos
        .get(vnum)
        .ok_or(MoveRefused::UnknownVnum(vnum))?;
    if proto.item_type == ITEM_QUEST && picker.rules.questing {
        return Err(MoveRefused::PickedUpWhileQuesting);
    }
    let mut records = Vec::new();
    let mut changes = Vec::new();
    let gone = MoveRecord::Ground(GroundRecord::Del { vid: ground.vid });
    let stackable = ground.item.flags & ITEM_FLAG_STACKABLE != 0
        && ground.item.anti_flags & ITEM_ANTIFLAG_STACK == 0;
    if stackable {
        let mut left = ground.item.count;
        for cell in merge_cells() {
            let pos = ItemPos::new(EWindows::Inventory as u8, cell);
            let Some(stack) = items.item_at(pos) else {
                continue;
            };
            if stack.vnum != vnum || stack.sockets != ground.item.sockets {
                continue;
            }
            let (id, held) = (stack.id, stack.count);
            let taken = picker.rules.count_limit.saturating_sub(held).min(left);
            left -= taken;
            let count = items
                .set_count(id, u32::from(held + taken))
                .map_err(MoveRefused::Count)?;
            let stack = items.item(id).ok_or(MoveRefused::Empty)?;
            records.push(MoveRecord::Item(ItemRecord::Update(stack.gc_item_update())));
            changes.push(ItemChange::Count { id, count });
            if left == 0 {
                ground.item.count = 0;
                records.push(MoveRecord::PickedUp { vnum });
                records.push(gone);
                return Ok(MoveDone {
                    kind: MoveKind::Merged,
                    records,
                    changes,
                });
            }
        }
        ground.item.count = left;
    }
    let size = ground.item.size;
    let cell = (0..CATEGORY_NUM)
        .filter(|bank| is_custom_category(proto, *bank))
        .find_map(|bank| items.find_free_custom_cell(bank, size))
        .or_else(|| items.find_free_inventory_cell(picker.rules.usable_cells, size));
    let Some(cell) = cell else {
        if changes.is_empty() {
            return Err(MoveRefused::NoRoomToPickUp);
        }
        records.push(MoveRecord::Notice("[LS;445]"));
        return Ok(MoveDone {
            kind: MoveKind::Declined,
            records,
            changes,
        });
    };
    let pos = ItemPos::new(EWindows::Inventory as u8, cell);
    let mut item = ground.item.clone();
    item.pos = pos;
    let _rolled = roll_sash(&mut item, proto, &mut *picker.dice);
    items.set(pos, &item).map_err(MoveRefused::Storage)?;
    // `AddToCharacter`'s highlight: on unless the picker was the last to hold it.
    let highlight = u8::from(ground.last_owner != picker.owner_id);
    records.push(gone);
    records.push(MoveRecord::Item(ItemRecord::Set(
        item.gc_item_set(pos, highlight),
    )));
    records.push(MoveRecord::PickedUp { vnum });
    changes.push(ItemChange::Created(item));
    ground.item.count = 0;
    Ok(MoveDone {
        kind: MoveKind::PickedUp,
        records,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::ItemIdRange;
    use gamedata::item_kind::{COSTUME_SASH, ITEM_COSTUME};
    use gamedata::item_proto::ItemProto;

    const INV: u8 = EWindows::Inventory as u8;

    const SWORD: u32 = 19;
    const ARROW: u32 = 8_000;
    const BANKED: u32 = 70_800;
    const SASH: u32 = 85_004;
    /// An `ITEM_QUEST`.
    const LETTER: u32 = 50_000;

    const RULES: MoveRules = MoveRules {
        count_limit: 200,
        usable_cells: 90,
        belt_grade: None,
        questing: false,
    };

    struct Fixed(u32);

    impl Dice for Fixed {
        fn random31(&mut self) -> u32 {
            self.0
        }
    }

    fn inv(cell: u16) -> ItemPos {
        ItemPos::new(INV, cell)
    }

    fn protos() -> ItemProtos {
        let mut sash = ItemProto::for_category_rule(SASH, ITEM_COSTUME, COSTUME_SASH);
        sash.values[0] = 4;
        ItemProtos::from_rows(vec![
            ItemProto::for_category_rule(SWORD, 1, 0),
            ItemProto::for_category_rule(ARROW, 1, 0),
            ItemProto::for_category_rule(BANKED, 0, 0),
            sash,
            ItemProto::for_category_rule(LETTER, ITEM_QUEST, 0),
        ])
    }

    fn arrows(id: u32, count: u16) -> Item {
        let mut item = Item::new(id, ARROW);
        item.count = count;
        item.flags = ITEM_FLAG_STACKABLE;
        item
    }

    fn holding(placed: &[(u16, Item)]) -> CharacterItems {
        let mut items = CharacterItems::new();
        for (cell, item) in placed {
            items.set(inv(*cell), item).expect("the fixture places");
        }
        items
    }

    fn ids() -> ItemIds {
        ItemIds::new(ItemIdRange::new(1000, 2000, 1000).expect("a valid range"))
    }

    const TO: DropAt = DropAt {
        vid: 3,
        x: 500,
        y: 700,
        owner_id: 42,
        questing: false,
        lands: true,
    };

    fn on_ground(item: Item, last_owner: u32) -> GroundItem {
        GroundItem {
            vid: 3,
            item,
            x: 500,
            y: 700,
            last_owner,
        }
    }

    fn pick(
        items: &mut CharacterItems,
        ground: &mut GroundItem,
        owner_id: u32,
        draw: u32,
    ) -> Result<MoveDone, MoveRefused> {
        let protos = protos();
        let mut dice = Fixed(draw);
        let mut picker = Picker {
            owner_id,
            rules: &RULES,
            protos: &protos,
            dice: &mut dice,
        };
        pickup_item(items, ground, &mut picker)
    }

    #[test]
    fn a_whole_item_leaves_the_cell_and_its_row() {
        let sword = Item::new(7, SWORD);
        let mut items = holding(&[(4, sword.clone())]);
        let (done, ground) = drop_item(&mut items, None, inv(4), 0, TO).expect("drops");
        assert_eq!(done.kind, MoveKind::Dropped);
        assert_eq!(
            done.records,
            vec![
                MoveRecord::QuickslotSync(QuickslotSync {
                    from: 4,
                    to: SyncTo::Delete
                }),
                MoveRecord::Item(ItemRecord::Set(gc_item_clear(inv(4)))),
                MoveRecord::Ground(GroundRecord::Add {
                    vid: 3,
                    vnum: SWORD,
                    x: 500,
                    y: 700
                }),
                MoveRecord::Notice(DROPPED_NOTICE),
            ]
        );
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert!(items.item_at(inv(4)).is_none());
        assert_eq!(ground.item.id, 7);
        assert_eq!(ground.last_owner, 42, "the dropper held it last");
    }

    #[test]
    fn a_count_over_the_stack_drops_the_whole_stack() {
        let mut items = holding(&[(4, arrows(7, 20))]);
        let (done, ground) = drop_item(&mut items, None, inv(4), 21, TO).expect("drops");
        assert_eq!(done.changes, vec![ItemChange::Destroyed { id: 7 }]);
        assert_eq!(ground.item.count, 20);
        let mut items = holding(&[(4, arrows(7, 20))]);
        let (_, ground) = drop_item(&mut items, None, inv(4), 20, TO).expect("drops");
        assert_eq!((ground.item.id, ground.item.count), (7, 20));
    }

    #[test]
    fn part_of_a_stack_becomes_a_new_item_nobody_held() {
        let mut stack = arrows(7, 20);
        stack.sockets[2] = 9;
        stack.attributes[0] = protocol::gc_item_window::ItemAttribute::new(1, 5);
        let mut items = holding(&[(4, stack)]);
        let mut ids = ids();
        let (done, ground) = drop_item(&mut items, Some(&mut ids), inv(4), 5, TO).expect("drops");
        let kept = items.item_at(inv(4)).expect("the stack stays").clone();
        assert_eq!(kept.count, 15);
        // The slots naming the cell are deleted even though part of the stack stays.
        assert_eq!(
            done.records[..2],
            [
                MoveRecord::QuickslotSync(QuickslotSync {
                    from: 4,
                    to: SyncTo::Delete
                }),
                MoveRecord::Item(ItemRecord::Update(kept.gc_item_update())),
            ]
        );
        assert_eq!(done.changes, vec![ItemChange::Count { id: 7, count: 15 }]);
        assert_eq!((ground.item.id, ground.item.count), (1000, 5));
        assert_eq!(ground.item.sockets[2], 9, "the sockets are copied");
        assert_eq!(
            ground.item.attributes[0].b_type, 0,
            "the attributes are not"
        );
        assert_eq!(ground.item.flags, ITEM_FLAG_STACKABLE);
        assert_eq!(ground.last_owner, 0);
    }

    #[test]
    fn a_drop_that_cannot_happen_changes_nothing() {
        let mut bound = Item::new(7, SWORD);
        bound.anti_flags = ITEM_ANTIFLAG_GIVE;
        let mut undroppable = Item::new(8, SWORD);
        undroppable.anti_flags = ITEM_ANTIFLAG_DROP;
        let mut items = holding(&[(4, bound), (5, undroppable), (6, arrows(9, 20))]);
        let before = items.clone();
        let cases = [
            (inv(4), 0, MoveRefused::Undroppable),
            (inv(5), 0, MoveRefused::Undroppable),
            (inv(7), 0, MoveRefused::Empty),
            (inv(6), 5, MoveRefused::NoItemIds),
            (ItemPos::new(INV, u16::MAX), 0, MoveRefused::InvalidSource),
            (
                ItemPos::new(EWindows::DragonSoulInventory as u8, 0),
                0,
                MoveRefused::NotPorted(Unported::SourceWindow(EWindows::DragonSoulInventory as u8)),
            ),
        ];
        for (at, count, reason) in cases {
            assert_eq!(drop_item(&mut items, None, at, count, TO), Err(reason));
        }
        assert_eq!(items, before);
        assert_eq!(MoveRefused::Undroppable.notice(), Some("[LS;442]"));
    }

    /// A drop where no sectree is keeps the item and the cell, and keeps the line an undroppable
    /// item is refused with, since that check comes first in legacy.
    #[test]
    fn a_drop_where_no_sectree_is_keeps_the_item() {
        let mut undroppable = Item::new(8, SWORD);
        undroppable.anti_flags = ITEM_ANTIFLAG_DROP;
        let mut items = holding(&[
            (4, Item::new(7, SWORD)),
            (5, undroppable),
            (6, arrows(9, 20)),
        ]);
        let before = items.clone();
        let nowhere = DropAt { lands: false, ..TO };
        let mut ids = ids();
        for (at, count, reason) in [
            (inv(4), 0, MoveRefused::NoSectree),
            (inv(6), 5, MoveRefused::NoSectree),
            (inv(5), 0, MoveRefused::Undroppable),
            (inv(7), 0, MoveRefused::Empty),
        ] {
            let dropped = drop_item(&mut items, Some(&mut ids), at, count, nowhere);
            assert_eq!(dropped, Err(reason), "{at:?}");
        }
        assert_eq!(items, before);
        assert_eq!(MoveRefused::NoSectree.notice(), None);
        assert!(drop_item(&mut items, Some(&mut ids), inv(6), 5, TO).is_ok());
    }

    #[test]
    fn an_item_offered_in_a_trade_is_not_dropped() {
        let mut items = holding(&[(4, Item::new(8, SWORD)), (6, arrows(9, 20))]);
        assert!(items.set_exchanging(8, true) && items.set_exchanging(9, true));
        let before = items.clone();
        for (at, count) in [(inv(4), 0), (inv(6), 5)] {
            let dropped = drop_item(&mut items, None, at, count, TO);
            assert_eq!(dropped, Err(MoveRefused::Exchanging), "{at:?}");
        }
        assert_eq!(items, before);
        assert!(items.set_exchanging(8, false));
        assert!(drop_item(&mut items, None, inv(4), 0, TO).is_ok());
    }

    /// `DropItem` (`G/char_item.cpp:7473-7486`): while a script waits nothing is dropped, silently,
    /// after the trade check and before the anti-flags.
    #[test]
    fn a_character_whose_script_waits_drops_nothing() {
        let mut bound = Item::new(7, SWORD);
        bound.anti_flags = ITEM_ANTIFLAG_GIVE;
        let mut items = holding(&[(4, bound), (5, Item::new(8, SWORD))]);
        assert!(items.set_exchanging(8, true));
        let before = items.clone();
        let questing = DropAt {
            questing: true,
            ..TO
        };
        let dropped = drop_item(&mut items, None, inv(4), 0, questing);
        assert_eq!(dropped, Err(MoveRefused::DroppedWhileQuesting));
        let offered = drop_item(&mut items, None, inv(5), 0, questing);
        assert_eq!(offered, Err(MoveRefused::Exchanging));
        assert_eq!(items, before);
        assert_eq!(MoveRefused::DroppedWhileQuesting.notice(), None);
        let dropped = drop_item(&mut items, None, inv(4), 0, TO);
        assert_eq!(dropped, Err(MoveRefused::Undroppable));
    }

    /// `PickupItem` (`@fixme150`, `G/char_item.cpp:7984-7993`): while a script waits a quest item
    /// stays on the ground, with a line; any other item is picked up.
    #[test]
    fn a_character_whose_script_waits_picks_up_no_quest_item() {
        let protos = protos();
        let rules = MoveRules {
            questing: true,
            ..RULES
        };
        let mut dice = Fixed(0);
        let mut picker = Picker {
            owner_id: 43,
            rules: &rules,
            protos: &protos,
            dice: &mut dice,
        };
        let mut items = CharacterItems::new();
        let mut letter = on_ground(Item::new(7, LETTER), 42);
        let refused = pickup_item(&mut items, &mut letter, &mut picker);
        assert_eq!(refused, Err(MoveRefused::PickedUpWhileQuesting));
        assert_eq!((letter.item.count, items.item(7)), (1, None));
        let notice = MoveRefused::PickedUpWhileQuesting.notice();
        assert_eq!(
            notice,
            Some("You cannot pickup this item if you're using quests")
        );
        let mut sword = on_ground(Item::new(8, SWORD), 42);
        let sword_moved = pickup_item(&mut items, &mut sword, &mut picker).expect("picks up");
        assert_eq!(sword_moved.kind, MoveKind::PickedUp);
        let letter_moved = pick(&mut items, &mut letter, 43, 0).expect("picks up");
        assert_eq!(letter_moved.kind, MoveKind::PickedUp);
    }

    /// A worn item is refused as not ported wherever it would land, so the warning names the
    /// missing path and not the point.
    #[test]
    fn a_worn_item_is_not_dropped_yet() {
        let worn = ItemPos::new(INV, INVENTORY_MAX_NUM);
        let mut items = CharacterItems::new();
        items.set(worn, &Item::new(7, SWORD)).expect("worn");
        let nowhere = DropAt { lands: false, ..TO };
        for to in [TO, nowhere] {
            assert_eq!(
                drop_item(&mut items, None, worn, 0, to),
                Err(MoveRefused::NotPorted(Unported::Equipment))
            );
        }
        assert!(items.item_at(worn).is_some());
    }

    #[test]
    fn a_picked_up_item_goes_to_the_first_free_cell_after_leaving_the_ground() {
        let mut items = holding(&[(0, Item::new(5, SWORD))]);
        let mut ground = on_ground(Item::new(7, SWORD), 42);
        let done = pick(&mut items, &mut ground, 43, 0).expect("picks up");
        assert_eq!(done.kind, MoveKind::PickedUp);
        let placed = items.item_at(inv(1)).expect("cell 1").clone();
        assert_eq!(placed.id, 7);
        assert_eq!(
            done.records,
            vec![
                MoveRecord::Ground(GroundRecord::Del { vid: 3 }),
                MoveRecord::Item(ItemRecord::Set(placed.gc_item_set(inv(1), 1))),
                MoveRecord::PickedUp { vnum: SWORD },
            ]
        );
        assert_eq!(done.changes, vec![ItemChange::Created(placed)]);
        assert_eq!(ground.item.count, 0);
    }

    #[test]
    fn the_last_holder_picks_up_without_the_highlight() {
        let mut items = CharacterItems::new();
        let mut ground = on_ground(Item::new(7, SWORD), 42);
        let done = pick(&mut items, &mut ground, 42, 0).expect("picks up");
        let placed = items.item_at(inv(0)).expect("cell 0").clone();
        assert_eq!(
            done.records[1],
            MoveRecord::Item(ItemRecord::Set(placed.gc_item_set(inv(0), 0)))
        );
    }

    #[test]
    fn a_banked_item_goes_to_its_bank_and_a_locked_page_is_never_offered() {
        let mut items = CharacterItems::new();
        let mut ground = on_ground(Item::new(7, BANKED), 0);
        pick(&mut items, &mut ground, 42, 0).expect("picks up");
        assert_eq!(
            items.cell_of(7),
            Some(inv(CUSTOM_INVENTORY_SLOT_START)),
            "bank 0 before the base inventory"
        );
        let full: Vec<(u16, Item)> = (0..RULES.usable_cells)
            .map(|cell| (cell, Item::new(100 + u32::from(cell), SWORD)))
            .collect();
        let mut items = holding(&full);
        let before = items.clone();
        let mut ground = on_ground(Item::new(7, SWORD), 0);
        assert_eq!(
            pick(&mut items, &mut ground, 42, 0),
            Err(MoveRefused::NoRoomToPickUp)
        );
        assert_eq!(items, before);
        assert_eq!(ground.item.count, 1, "the item stays on the ground");
    }

    #[test]
    fn a_stack_is_topped_up_in_cell_order_and_taken_whole_leaves_the_ground() {
        let mut other_sockets = arrows(5, 10);
        other_sockets.sockets[0] = 1;
        let mut items = holding(&[
            (0, other_sockets),
            (1, arrows(6, 190)),
            (2, Item::new(9, SWORD)),
            (3, arrows(8, 10)),
        ]);
        let mut ground = on_ground(arrows(7, 30), 0);
        let done = pick(&mut items, &mut ground, 42, 0).expect("merges");
        assert_eq!(done.kind, MoveKind::Merged);
        let first = items.item(6).expect("held").clone();
        let second = items.item(8).expect("held").clone();
        assert_eq!((first.count, second.count), (200, 30));
        assert_eq!(items.item(5).expect("held").count, 10, "other sockets");
        assert_eq!(
            done.records,
            vec![
                MoveRecord::Item(ItemRecord::Update(first.gc_item_update())),
                MoveRecord::Item(ItemRecord::Update(second.gc_item_update())),
                MoveRecord::PickedUp { vnum: ARROW },
                MoveRecord::Ground(GroundRecord::Del { vid: 3 }),
            ]
        );
        assert_eq!(
            done.changes,
            vec![
                ItemChange::Count { id: 6, count: 200 },
                ItemChange::Count { id: 8, count: 30 },
            ]
        );
        assert_eq!(ground.item.count, 0);
    }

    #[test]
    fn a_stack_two_cells_tall_is_topped_up_once() {
        let mut tall = arrows(6, 199);
        tall.size = 2;
        let mut items = holding(&[(0, tall)]);
        let mut ground = on_ground(arrows(7, 5), 0);
        let done = pick(&mut items, &mut ground, 42, 0).expect("picks up");
        assert_eq!(done.kind, MoveKind::PickedUp);
        assert_eq!(done.changes[0], ItemChange::Count { id: 6, count: 200 });
        assert_eq!(done.changes.len(), 2, "one merge, then the rest set");
        assert_eq!(items.item_at(inv(1)).map(|item| item.count), Some(4));
    }

    #[test]
    fn a_full_stack_is_updated_and_the_rest_goes_to_a_free_cell() {
        let mut items = holding(&[(0, arrows(6, 200))]);
        let mut ground = on_ground(arrows(7, 30), 0);
        let done = pick(&mut items, &mut ground, 42, 0).expect("picks up");
        assert_eq!(done.kind, MoveKind::PickedUp);
        assert_eq!(done.changes[0], ItemChange::Count { id: 6, count: 200 });
        assert_eq!(items.item_at(inv(1)).map(|item| item.count), Some(30));
    }

    #[test]
    fn a_stack_merged_in_part_with_no_free_cell_keeps_the_rest_on_the_ground() {
        let mut full: Vec<(u16, Item)> = (1..RULES.usable_cells)
            .map(|cell| (cell, Item::new(100 + u32::from(cell), SWORD)))
            .collect();
        full.push((0, arrows(6, 190)));
        let mut items = holding(&full);
        let mut ground = on_ground(arrows(7, 30), 0);
        let done = pick(&mut items, &mut ground, 42, 0).expect("merges in part");
        assert_eq!(done.kind, MoveKind::Declined);
        assert_eq!(done.records.last(), Some(&MoveRecord::Notice("[LS;445]")));
        assert_eq!(done.changes, vec![ItemChange::Count { id: 6, count: 200 }]);
        assert_eq!(ground.item.count, 20);
    }

    #[test]
    fn a_stack_that_may_not_merge_goes_whole_to_a_free_cell() {
        let mut unstackable = arrows(7, 30);
        unstackable.anti_flags = ITEM_ANTIFLAG_STACK;
        let mut items = holding(&[(0, arrows(6, 10))]);
        let mut ground = on_ground(unstackable, 0);
        pick(&mut items, &mut ground, 42, 0).expect("picks up");
        assert_eq!(items.item(6).map(|item| item.count), Some(10));
        assert_eq!(items.cell_of(7), Some(inv(1)));
    }

    #[test]
    fn a_tall_stack_is_topped_up_once() {
        let mut tall = arrows(6, 10);
        tall.size = 2;
        let mut items = holding(&[(0, tall)]);
        let mut ground = on_ground(arrows(7, 30), 0);
        let done = pick(&mut items, &mut ground, 42, 0).expect("merges");
        assert_eq!(items.item(6).map(|item| item.count), Some(40));
        assert_eq!(done.changes.len(), 1);
    }

    #[test]
    fn a_sash_rolls_its_absorption_when_it_is_picked_up() {
        let mut items = CharacterItems::new();
        let mut ground = on_ground(Item::new(7, SASH), 0);
        let done = pick(&mut items, &mut ground, 42, 8).expect("picks up");
        let placed = items.item(7).expect("held").clone();
        assert_eq!(placed.sockets[0], 19);
        assert_eq!(done.changes, vec![ItemChange::Created(placed)]);
    }

    #[test]
    fn an_item_with_no_prototype_is_left_on_the_ground() {
        let mut items = CharacterItems::new();
        let mut ground = on_ground(Item::new(7, 1), 0);
        assert_eq!(
            pick(&mut items, &mut ground, 42, 0),
            Err(MoveRefused::UnknownVnum(1))
        );
    }
}
