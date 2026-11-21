//! The GM's rod and pick lines: `/refine_rod`, `/refine_pick` and `/max_pick`
//! (`ACMD(do_refine_rod)`, `do_refine_pick`, `do_max_pick`, `G/cmd_gm.cpp:2181-2222`).
//!
//! `RealRefineRod` (`G/fishing.cpp:861-928`) and `RealRefinePick` (`G/mining.cpp:234-303`)
//! swap the named cell's tool for its next vnum on a draw of `number(1, 100)` against
//! the proto's `VALUE3`, and for the fallback vnum (`VALUE4`) on a miss; both protos on
//! the owner's data refine to their own line's next vnum and fall back to themselves.
//! A tool whose socket 0 holds less than the proto's `VALUE2` never refines. `max_pick`
//! is `CHEAT_MAX_PICK` (`G/mining.cpp:304-315`): socket 0 takes `VALUE2` and the line
//! tells it. Mine's `ENABLE_PICKAXE_RENEWAL` and rod's `ENABLE_FISHINGROD_RENEWAL` are
//! undefined, so the miss swaps rather than taxing the socket.
//!
//! The refine and pick logs (`RefineLog`, `ItemLog`) are `sys.log`'s, which owns no sink,
//! so they are not written. The hack lines (`ROD_HACK`, `PICK_HACK`) go the same way.
//!
//! `G/` = `server/server/game/`.

use common::item_slots::EWindows;
use gamedata::item_kind::{ITEM_PICK, ITEM_ROD};
use gamedata::item_proto::ItemProtos;
use protocol::item_pos::ItemPos;

use super::dice::{number, Dice};
use super::item_move::{ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord};
use super::items::CharacterItems;
use super::quickslot::{QuickslotSync, SyncTo};
use crate::item::{gc_item_clear, Item, ItemIds};

/// Which tool line runs: `RealRefineRod`'s rod arm, `RealRefinePick`'s pick arm with its
/// `CHEAT_MAX_PICK` first, the max alone, or `RealRefinePick` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRefine {
    /// `RefineRod`: the cell must hold a rod.
    Rod,
    /// `RefinePick`: the cell must hold a pick; the training maxes before the refine.
    Pick,
    /// `MaxPick`: the cell must hold a pick; the training maxes and nothing refines.
    Max,
    /// `RealRefinePick` alone, as the quest's `__refine_pick` runs it: the cell must hold a
    /// pick, and nothing maxes its training.
    PickAlone,
}

/// What `RealRefineRod` and `RealRefinePick` answer (`G/fishing.cpp:861-928`,
/// `G/mining.cpp:234-303`), which the quest's `__fish_real_refine_rod` and `__refine_pick`
/// return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutcome {
    /// 2: no tool, the wrong type, a worn or untrained tool, or no item to make; also the
    /// max alone, which refines nothing.
    Refused,
    /// 1: the draw hit and the refined vnum stands in the cell.
    Refined,
    /// 0: the draw missed and the `VALUE4` fallback stands in the cell.
    Fallback,
}

impl ToolOutcome {
    /// The number the legacy function returns.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Refused => 2,
            Self::Refined => 1,
            Self::Fallback => 0,
        }
    }
}

/// `str_to_number(BYTE)`'s low byte of the line's word (`C/utils.h:20-26`): `strtoul`
/// base 10, which stops at the first non-digit, empties to the unchanged cell 0, and
/// keeps only the low byte.
#[must_use]
pub fn refine_cell(word: &[u8]) -> u8 {
    let mut value: u64 = 0;
    for byte in word {
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit < 10) else {
            break;
        };
        value = value.saturating_mul(10).saturating_add(u64::from(digit));
    }
    value.to_le_bytes()[0]
}

/// One tool line for the character's storage: the named inventory cell's tool refines,
/// maxes, or both, with the new tool in the same cell and the old id destroyed. Answers the
/// step and what the refine came to.
#[must_use]
pub fn refine_tool(
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    protos: &ItemProtos,
    dice: &mut dyn Dice,
    cell: u8,
    line: ToolRefine,
) -> (MoveDone, ToolOutcome) {
    let mut done = MoveDone {
        kind: MoveKind::Refined,
        records: Vec::new(),
        changes: Vec::new(),
    };
    let outcome = run_tool_line(&mut done, items, ids, protos, dice, cell, line)
        .unwrap_or(ToolOutcome::Refused);
    (done, outcome)
}

/// The body of [`refine_tool`], filling `done`; `None` is a refusal.
fn run_tool_line(
    done: &mut MoveDone,
    items: &mut CharacterItems,
    ids: Option<&mut ItemIds>,
    protos: &ItemProtos,
    dice: &mut dyn Dice,
    cell: u8,
    line: ToolRefine,
) -> Option<ToolOutcome> {
    let pos = ItemPos::new(EWindows::Inventory as u8, u16::from(cell));
    let held = items.item_at(pos)?;
    let (id, vnum) = (held.id, held.vnum);
    let proto = protos.get(vnum)?;
    let wanted = match line {
        ToolRefine::Rod => ITEM_ROD,
        ToolRefine::Pick | ToolRefine::Max | ToolRefine::PickAlone => ITEM_PICK,
    };
    if proto.item_type != wanted {
        return None;
    }
    // `RefinableRod`'s equipped arm (`G/fishing.cpp:851-857`) and pick's
    // (`G/mining.cpp:257-260`): a worn tool never refines or maxes.
    if items
        .item(id)
        .is_some_and(|item| item.window() != EWindows::Inventory as u8)
    {
        return None;
    }
    let need = proto.values[2];
    if line == ToolRefine::Pick || line == ToolRefine::Max {
        // `CHEAT_MAX_PICK` (`G/mining.cpp:304-315`): socket 0 takes the max, the
        // cell updates, and the line tells the new training.
        items.set_socket(id, 0, need).ok()?;
        if let Some(item) = items.item(id) {
            let update = ItemRecord::Update(item.gc_item_update());
            done.records.push(MoveRecord::Item(update));
            done.changes.push(ItemChange::Sockets {
                id,
                sockets: item.sockets,
            });
        }
    }
    if line == ToolRefine::Max {
        return None;
    }
    // `RefinableRod` (`G/fishing.cpp:850-859`) and `Pick_Refinable`
    // (`G/mining.cpp:216-224`): socket 0 must hold the max (the rod asks for equality,
    // the pick refuses below).
    let trained = items.item(id).map_or(0, |item| item.sockets[0]);
    let ready = if line == ToolRefine::Rod {
        trained == need
    } else {
        trained >= need
    };
    if !ready {
        return None;
    }
    // The draw (`number(1, 100) <= GetValue(3)`): the refined vnum on a hit, `VALUE4`
    // on a miss.
    let (next, outcome) = if number(dice, 1, 100) <= proto.values[3] {
        (proto.refined_vnum, ToolOutcome::Refined)
    } else {
        (
            u32::try_from(proto.values[4]).unwrap_or(0),
            ToolOutcome::Fallback,
        )
    };
    let made = protos.get(next)?;
    let fresh = ids?.allocate().ok()?;
    // `RemoveItem` (`G/item_manager.cpp:566-598`): the slots naming the cell are
    // deleted, then the tool leaves the storage and the client's cell.
    let sync = QuickslotSync {
        from: pos.cell,
        to: SyncTo::Delete,
    };
    done.records.push(MoveRecord::QuickslotSync(sync));
    items.release(id).ok()?;
    done.records
        .push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
    done.changes.push(ItemChange::Destroyed { id });
    let mut item = Item::new(fresh, next);
    let size = u8::try_from(made.size).ok()?;
    item.set_size(size).ok()?;
    item.flags = made.flags;
    item.anti_flags = made.anti_flags;
    item.pos = pos;
    items.set(pos, &item).ok()?;
    done.records
        .push(MoveRecord::Item(ItemRecord::Set(item.gc_item_set(pos, 0))));
    done.changes.push(ItemChange::Created(item));
    Some(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::ItemIdRange;
    use gamedata::item_proto::ItemProto;

    const INV: u8 = EWindows::Inventory as u8;

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

    fn tool(vnum: u32, item_type: i32, refined: u32, values: [i32; 6]) -> ItemProto {
        let mut proto = ItemProto::for_category_rule(vnum, item_type, 0);
        proto.refined_vnum = refined;
        proto.values = values;
        proto.size = 1;
        proto
    }

    fn rod() -> ItemProto {
        tool(27400, ITEM_ROD, 27410, [10, 5, 10, 100, 27400, 0])
    }

    fn pick() -> ItemProto {
        tool(29101, ITEM_PICK, 29102, [0, 1, 6000, 100, 29101, 0])
    }

    fn protos() -> ItemProtos {
        let next_rod = tool(27410, ITEM_ROD, 27420, [20, 10, 20, 39, 27410, 10]);
        let next_pick = tool(29102, ITEM_PICK, 29103, [10, 1, 12000, 39, 29102, 10]);
        ItemProtos::from_rows(vec![rod(), pick(), next_rod, next_pick])
    }

    fn placed(items: &mut CharacterItems, id: u32, vnum: u32, cell: u16, socket: i32) {
        let pos = ItemPos::new(INV, cell);
        let mut item = Item::new(id, vnum);
        item.pos = pos;
        item.sockets[0] = socket;
        items.set(pos, &item).expect("a free cell takes the tool");
    }

    #[test]
    fn the_cell_keeps_the_low_byte_and_stops_at_a_non_digit() {
        assert_eq!(refine_cell(b""), 0);
        assert_eq!(refine_cell(b"7"), 7);
        assert_eq!(refine_cell(b"12x"), 12);
        assert_eq!(refine_cell(b"256"), 0);
        assert_eq!(refine_cell(b"257"), 1);
    }

    #[test]
    fn a_size3_rod_swaps_in_place() {
        let mut old = rod();
        old.size = 3;
        let mut next = tool(27410, ITEM_ROD, 27420, [20, 10, 20, 39, 27410, 10]);
        next.size = 3;
        let table = ItemProtos::from_rows(vec![old, next]);
        let mut items = CharacterItems::new();
        let pos = ItemPos::new(INV, 3);
        let mut item = Item::new(1, 27400);
        item.pos = pos;
        item.set_size(3).expect("a size");
        item.sockets[0] = 10;
        items.set(pos, &item).expect("a placed rod");
        let mut ids = ItemIds::new(ItemIdRange::new(2, 1_000_000, 2).expect("a usable range"));
        let mut dice = Script::new(vec![0]);
        let (done, _) = refine_tool(
            &mut items,
            Some(&mut ids),
            &table,
            &mut dice,
            3,
            ToolRefine::Rod,
        );
        assert_eq!(done.changes.len(), 2, "the swap stores both halves");
        assert_eq!(items.item_at(pos).map(|item| item.vnum), Some(27410));
    }

    #[test]
    fn a_trained_rod_swaps_to_its_next_vnum() {
        let protos = protos();
        let mut items = CharacterItems::new();
        placed(&mut items, 1, 27400, 3, 10);
        let mut ids = ItemIds::new(ItemIdRange::new(2, 1_000_000, 2).expect("a usable range"));
        let mut dice = Script::new(vec![1]);
        let (done, outcome) = refine_tool(
            &mut items,
            Some(&mut ids),
            &protos,
            &mut dice,
            3,
            ToolRefine::Rod,
        );
        assert_eq!(done.kind, MoveKind::Refined);
        assert_eq!(outcome.code(), 1, "a hit answers 1");
        let pos = ItemPos::new(INV, 3);
        assert_eq!(items.item_at(pos).map(|item| item.vnum), Some(27410));
        assert!(matches!(
            done.changes.as_slice(),
            [ItemChange::Destroyed { .. }, ItemChange::Created(_)]
        ));
    }

    #[test]
    fn an_untrained_rod_changes_nothing() {
        let protos = protos();
        let mut items = CharacterItems::new();
        placed(&mut items, 1, 27400, 3, 9);
        let mut ids = ItemIds::new(ItemIdRange::new(2, 1_000_000, 2).expect("a usable range"));
        let mut dice = Script::new(vec![1]);
        let (done, outcome) = refine_tool(
            &mut items,
            Some(&mut ids),
            &protos,
            &mut dice,
            3,
            ToolRefine::Rod,
        );
        assert_eq!(outcome.code(), 2, "an untrained rod answers 2");
        assert!(done.records.is_empty());
        assert!(done.changes.is_empty());
        let pos = ItemPos::new(INV, 3);
        assert_eq!(items.item_at(pos).map(|item| item.vnum), Some(27400));
    }

    #[test]
    fn a_miss_swaps_to_the_fallback_vnum() {
        let protos = protos();
        let mut items = CharacterItems::new();
        placed(&mut items, 1, 27400, 3, 10);
        let mut ids = ItemIds::new(ItemIdRange::new(2, 1_000_000, 2).expect("a usable range"));
        let mut dice = Script::new(vec![98]);
        let mut weak = rod();
        weak.values[3] = 1;
        let table = ItemProtos::from_rows(vec![
            weak,
            tool(27410, ITEM_ROD, 27420, [20, 10, 20, 39, 27410, 10]),
        ]);
        let _ = protos;
        let (done, outcome) = refine_tool(
            &mut items,
            Some(&mut ids),
            &table,
            &mut dice,
            3,
            ToolRefine::Rod,
        );
        let pos = ItemPos::new(INV, 3);
        assert_eq!(items.item_at(pos).map(|item| item.vnum), Some(27400));
        assert_eq!(done.changes.len(), 2);
        assert_eq!(outcome.code(), 0, "a miss answers 0");
    }

    #[test]
    fn a_pick_maxes_then_refines_and_a_max_only_maxes() {
        let protos = protos();
        let mut items = CharacterItems::new();
        placed(&mut items, 1, 29101, 5, 0);
        let mut ids = ItemIds::new(ItemIdRange::new(2, 1_000_000, 2).expect("a usable range"));
        let mut dice = Script::new(vec![1]);
        let (done, _) = refine_tool(
            &mut items,
            Some(&mut ids),
            &protos,
            &mut dice,
            5,
            ToolRefine::Pick,
        );
        let pos = ItemPos::new(INV, 5);
        assert_eq!(items.item_at(pos).map(|item| item.vnum), Some(29102));
        assert!(matches!(
            done.changes.as_slice(),
            [
                ItemChange::Sockets { .. },
                ItemChange::Destroyed { .. },
                ItemChange::Created(_)
            ]
        ));
        placed(&mut items, 9, 29101, 6, 0);
        let mut dice = Script::new(vec![1]);
        let (done, _) = refine_tool(
            &mut items,
            Some(&mut ids),
            &protos,
            &mut dice,
            6,
            ToolRefine::Max,
        );
        let alone = ItemPos::new(INV, 6);
        assert_eq!(items.item_at(alone).map(|item| item.vnum), Some(29101));
        assert_eq!(items.item_at(alone).map(|item| item.sockets[0]), Some(6000));
        assert_eq!(done.changes.len(), 1);
    }

    #[test]
    fn a_pick_alone_refines_only_once_trained_and_never_maxes() {
        let protos = protos();
        let mut items = CharacterItems::new();
        placed(&mut items, 1, 29101, 5, 5999);
        let mut ids = ItemIds::new(ItemIdRange::new(2, 1_000_000, 2).expect("a usable range"));
        let mut dice = Script::new(vec![1]);
        let (done, outcome) = refine_tool(
            &mut items,
            Some(&mut ids),
            &protos,
            &mut dice,
            5,
            ToolRefine::PickAlone,
        );
        assert_eq!(outcome, ToolOutcome::Refused);
        assert!(done.changes.is_empty(), "nothing maxes the training");
        let pos = ItemPos::new(INV, 5);
        assert_eq!(items.item_at(pos).map(|item| item.sockets[0]), Some(5999));
        placed(&mut items, 9, 29101, 6, 6000);
        let (done, outcome) = refine_tool(
            &mut items,
            Some(&mut ids),
            &protos,
            &mut dice,
            6,
            ToolRefine::PickAlone,
        );
        assert_eq!(outcome, ToolOutcome::Refined);
        assert!(matches!(
            done.changes.as_slice(),
            [ItemChange::Destroyed { .. }, ItemChange::Created(_)]
        ));
    }
}
