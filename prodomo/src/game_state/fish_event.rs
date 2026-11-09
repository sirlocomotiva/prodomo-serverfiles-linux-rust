//! The jigsaw fish event (`ENABLE_FISH_EVENT`, `G/char.cpp:10880-11121`).
//!
//! A box (25106) draws a random shape 1-6 and a special box (25107) attaches shape 7,
//! each spending one box and raising the use count; the client lands the attached shape
//! on the 24-cell board (`valid_position`'s tables at `:10963-10971`), and a full board
//! pays a gift box (25113 at 10 boxes or fewer, 25112 at 24 or fewer, else 25111) with
//! 30000 exp, then clears the board and the count.
//!
//! The board and the count are the character's (`m_fishSlots`, `m_dwFishUseCount`,
//! `G/char.h:1045,2063`): the load installs them, the save writes them, and the
//! login report (`FishEventGeneralInfo`, `:10880-10913`) sends the enable flag beside
//! the count, then one `SHAPE_ADD` per main cell. The attached shape is memory-only
//! (`m_bFishAttachedShape`): the save never writes it, so a relogin drops what was held.
//!
//! # Not ported
//!
//! Nothing. The two CG arms, the login report, the reward and the save round-trip are
//! all here.
//!
//! # Defects not reproduced
//!
//! - A shape past 7 is refused (`lastAttachedShape > ITEM_FISH_EVENT_BOX_SPECIAL`,
//!   `:11086`): the guard compares a shape to the special box's *vnum* (25107), so every
//!   byte passes it. The Rewrite compares to shape 7, the highest the tables name.

use common::vid::Vid;
use db::items::RowChange;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_vid::{GcFishEventInfo, GcFishEventInfoSubheader};
use protocol::item_pos::ItemPos;
use world::character::{number, MoveDone, MoveKind, MoveRefused, Unported};

use super::GameState;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// `FISH_EVENT_SLOTS_NUM` (`C/length.h:110`): the 24 cells of the jigsaw board.
pub const FISH_EVENT_SLOTS: usize = 24;
/// `ITEM_FISH_EVENT_BOX` (`C/length.h:111`): the box that draws a random shape 1-6.
pub const FISH_EVENT_BOX: u32 = 25106;
/// `ITEM_FISH_EVENT_BOX_SPECIAL` (`C/length.h:112`): the box that attaches shape 7.
pub const FISH_EVENT_BOX_SPECIAL: u32 = 25107;
/// `FISH_EVENT_SHAPE_7` (`C/length.h:194`): the highest shape the tables name.
pub const FISH_EVENT_SHAPE_MAX: u8 = 7;
/// The completion reward at 10 boxes or fewer (`G/char.cpp:11053`).
const REWARD_TEN: u32 = 25113;
/// The completion reward at 24 boxes or fewer.
const REWARD_MID: u32 = 25112;
/// The completion reward past 24 boxes.
const REWARD_REST: u32 = 25111;
/// `PointChange(POINT_EXP, 30000)` on completion (`:11066`).
const COMPLETION_EXP: i64 = 30_000;

/// One fish-event request, after the window byte's check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FishEventStep {
    /// `FISH_EVENT_SUBHEADER_BOX_USE` with the box's position.
    BoxUse(ItemPos),
    /// `FISH_EVENT_SUBHEADER_SHAPE_ADD` with the board cell.
    ShapeAdd(u8),
}

/// One board cell: whether a shape was placed here, and the shape on it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FishCell {
    /// `bIsMain`: the cell the shape was placed at.
    pub is_main: bool,
    /// `bShape`: 0 for empty, else 1 to 7.
    pub shape: u8,
}

/// A character's fish-event board: its cells, its use count, and the held shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FishBoard {
    /// `m_fishSlots`: the 24 cells.
    pub cells: [FishCell; FISH_EVENT_SLOTS],
    /// `m_dwFishUseCount`: the boxes used on this board.
    pub use_count: u32,
    /// `m_bFishAttachedShape`: the held shape. Memory-only: the save never writes it.
    pub attached: u8,
}

impl Default for FishBoard {
    fn default() -> Self {
        Self {
            cells: [FishCell::default(); FISH_EVENT_SLOTS],
            use_count: 0,
            attached: 0,
        }
    }
}

/// A `GC_FISH_EVENT_INFO` record.
fn fish_event_record(subheader: GcFishEventInfoSubheader, first: u32, second: u32) -> Vec<u8> {
    let mut record = Vec::with_capacity(GcFishEventInfo::WIRE_SIZE);
    GcFishEventInfo {
        subheader,
        first,
        second,
    }
    .encode_into(&mut record);
    record
}

/// `FishEventIsValidPosition` (`G/char.cpp:10961-10999`): whether `shape` lands at `pos`.
/// The table names each shape's legal heads and the six offsets every head checks; a head
/// whose row repeats an offset checks that cell twice, exactly as legacy does.
pub fn valid_position(cells: &[FishCell; FISH_EVENT_SLOTS], pos: u8, shape: u8) -> bool {
    const POSITIONS: [(u8, usize, [usize; 6]); 7] = [
        (1, 12, [0, 6, 12, 0, 0, 0]),
        (2, 24, [0, 0, 0, 0, 0, 0]),
        (3, 17, [0, 6, 7, 0, 0, 0]),
        (4, 17, [0, 1, 7, 0, 0, 0]),
        (5, 17, [0, 1, 6, 7, 0, 0]),
        (6, 16, [0, 1, 7, 8, 0, 0]),
        (7, 16, [0, 1, 2, 6, 7, 8]),
    ];
    let Some((_, heads, offsets)) = POSITIONS.iter().find(|row| row.0 == shape) else {
        return false;
    };
    (0..*heads).any(|head| {
        head == usize::from(pos) && offsets.iter().all(|offset| cells[head + offset].shape == 0)
    })
}

/// `FishEventPlaceShape` (`G/char.cpp:11001-11037`): stamp `shape` at `pos`. Offset 0 is
/// the main cell; later zeroes are skipped, so a shape-1 stamp writes its main cell once.
pub fn place_shape(cells: &mut [FishCell; FISH_EVENT_SLOTS], pos: u8, shape: u8) {
    const SHAPES: [(u8, [usize; 6]); 7] = [
        (1, [0, 6, 12, 0, 0, 0]),
        (2, [0, 0, 0, 0, 0, 0]),
        (3, [0, 6, 7, 0, 0, 0]),
        (4, [0, 1, 7, 0, 0, 0]),
        (5, [0, 1, 6, 7, 0, 0]),
        (6, [0, 1, 7, 8, 0, 0]),
        (7, [0, 1, 2, 6, 7, 8]),
    ];
    let Some(offsets) = SHAPES.iter().find(|row| row.0 == shape).map(|row| row.1) else {
        return;
    };
    for (slot, offset) in offsets.iter().enumerate() {
        if slot > 0 && *offset == 0 {
            continue;
        }
        let at = usize::from(pos) + offset;
        cells[at] = FishCell {
            is_main: *offset == 0,
            shape,
        };
    }
}

/// The completion reward for `count` used boxes (`G/char.cpp:11053`).
const fn reward_for(count: u32) -> u32 {
    if count <= 10 {
        REWARD_TEN
    } else if count <= 24 {
        REWARD_MID
    } else {
        REWARD_REST
    }
}

/// `number(1, 6)` for a normal box (`G/char.cpp:11046`): the drawn shape 1-6.
pub fn draw_shape(dice: &mut dyn world::character::Dice) -> u8 {
    u8::try_from(number(dice, 1, 6)).unwrap_or(1)
}

impl GameState {
    /// The fish-event board the character online under `vid` plays on, installed at
    /// admission and empty for a character that entered without one.
    fn fish_board_mut(&mut self, vid: Vid) -> &mut FishBoard {
        self.fish_boards.entry(vid).or_default()
    }

    /// Install the stored board `cells` with `use_count` for the character under `vid`:
    /// the load of `fishSlots` and `fishEventUseCount` (`G/char.cpp:2280-2286`). A row
    /// past cell 23 or with no shape is dropped, as the width checks the store never
    /// lets through would have; the held shape stays 0.
    pub(super) fn install_fish_board(
        &mut self,
        vid: Vid,
        cells: &[db::fish_event::StoredFishCell],
        use_count: u32,
    ) {
        let mut board = FishBoard {
            use_count,
            ..FishBoard::default()
        };
        for stored in cells {
            if usize::from(stored.cell) >= FISH_EVENT_SLOTS || stored.shape == 0 {
                continue;
            }
            board.cells[usize::from(stored.cell)] = FishCell {
                is_main: stored.is_main,
                shape: stored.shape,
            };
        }
        let _installed = self.fish_boards.insert(vid, board);
    }

    /// The board the character under `vid` holds, as the set cells in cell order with
    /// the use count, which a save writes (`G/char.cpp:1613-1614`). Empty for a
    /// character that entered without one.
    pub(super) fn kept_fish_board(&self, vid: Vid) -> (Vec<db::fish_event::StoredFishCell>, u32) {
        let Some(board) = self.fish_boards.get(&vid) else {
            return (Vec::new(), 0);
        };
        let cells = board
            .cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.shape != 0)
            .map(|(cell, placed)| db::fish_event::StoredFishCell {
                cell: u8::try_from(cell).unwrap_or(u8::MAX),
                is_main: placed.is_main,
                shape: placed.shape,
            })
            .collect();
        (cells, board.use_count)
    }

    /// `FishEventGeneralInfo` (`G/char.cpp:10880-10913`): the login report. The enable
    /// record carries the `enable_fish_event` flag beside the use count; a count of 0
    /// clears the board first, and one `SHAPE_ADD` per main cell follows.
    ///
    /// The caller holds the burst, so the lines are answered rather than sent.
    pub(super) fn fish_event_report(&mut self, vid: Vid) -> Vec<Vec<u8>> {
        let flag = u32::try_from(self.event_rate(b"enable_fish_event").max(0)).unwrap_or(u32::MAX);
        let board = self.fish_board_mut(vid);
        if board.use_count == 0 {
            for cell in &mut board.cells {
                *cell = FishCell::default();
            }
        }
        let mut lines = vec![fish_event_record(
            GcFishEventInfoSubheader::Enable,
            flag,
            board.use_count,
        )];
        lines.extend(
            board
                .cells
                .iter()
                .enumerate()
                .filter(|(_, cell)| cell.is_main)
                .map(|(cell, placed)| {
                    let at = u32::try_from(cell).unwrap_or(u32::from(u8::MAX));
                    fish_event_record(
                        GcFishEventInfoSubheader::ShapeAdd,
                        at,
                        u32::from(placed.shape),
                    )
                }),
        );
        lines
    }

    /// One `CG_FISH_EVENT_SEND` of the character online under `vid`: a box use or a
    /// shape add. Every record reaches the client through the world, in legacy's order;
    /// the answer carries the rows to store.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when the world does not hold the character.
    pub fn fish_event(
        &mut self,
        vid: Vid,
        step: FishEventStep,
        mover: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?
            .player_id();
        match step {
            FishEventStep::BoxUse(at) => {
                Ok(self.fish_use_box(vid, at, mover).unwrap_or_else(|| {
                    Self::empty_answer(&self.protos, &self.locale, pid, vid, mover)
                }))
            }
            FishEventStep::ShapeAdd(pos) => {
                let changes = if self.fish_add_shape(vid, pos, mover) {
                    self.finish_fish_shape(vid, pid, mover)
                } else {
                    Vec::new()
                };
                let mut answer = Self::empty_answer(&self.protos, &self.locale, pid, vid, mover);
                answer.changes = changes;
                Ok(answer)
            }
        }
    }

    /// An answer that carries nothing: a refused or record-only step.
    fn empty_answer(
        protos: &gamedata::item_proto::ItemProtos,
        locale: &std::sync::Arc<gamedata::locale_string::LocaleStrings>,
        pid: u32,
        vid: Vid,
        mover: Mover,
    ) -> MovedItems {
        MovedItems::new(
            pid,
            vid.raw(),
            MoveDone {
                kind: MoveKind::Socketed,
                records: Vec::new(),
                changes: Vec::new(),
            },
            mover,
            protos,
            locale,
        )
    }
    /// `FishEventUseBox` (`G/char.cpp:10915-10959`): attach the box's shape, spending one
    /// box and raising the count, then answer the `BOX_USE` record. Anything else at the
    /// cell is told `You can not use this item here`; a gated character, a bad cell, an
    /// empty cell and an offered box end silently. Answers the spend step, if one ran.
    fn fish_use_box(&mut self, vid: Vid, at: ItemPos, mover: Mover) -> Option<MovedItems> {
        // `CanHandleItem`, the windows this build has, as `destroy_item` gates it.
        if self.sash_windows.contains_key(&vid)
            || self.changelook_windows.contains(&vid)
            || self.aura_windows.contains_key(&vid)
            || self.under_refine(vid)
        {
            return None;
        }
        if at.window_type != common::item_slots::EWindows::Inventory as u8
            || !world::character::is_valid_item_position(at)
        {
            return None;
        }
        let vnum = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| {
                let item = character.items().item_at(at)?;
                (!character.items().is_exchanging(item.id)).then_some(item.vnum)
            })?;
        if vnum != FISH_EVENT_BOX && vnum != FISH_EVENT_BOX_SPECIAL {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"You can not use this item here",
                &[],
            );
            return None;
        }
        if vnum == FISH_EVENT_BOX {
            let shape = draw_shape(&mut self.dice);
            self.fish_board_mut(vid).attached = shape;
        } else {
            self.fish_board_mut(vid).attached = FISH_EVENT_SHAPE_MAX;
        }
        self.fish_board_mut(vid).use_count = self.fish_board_mut(vid).use_count.saturating_add(1);
        let done = self
            .run_item_step(vid, mover, |items, _ids, _rules, _gear, protos| {
                let item = items
                    .item_at(at)
                    .cloned()
                    .ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
                let proto = protos
                    .get(item.vnum)
                    .ok_or(MoveRefused::UnknownVnum(item.vnum))?;
                world::character::spend_one(items, &item, proto)
            })
            .ok()?;
        let (attached, use_count) = {
            let board = self.fish_board_mut(vid);
            (board.attached, board.use_count)
        };
        let _sent = self.write_to_client(
            vid,
            fish_event_record(
                GcFishEventInfoSubheader::BoxUse,
                u32::from(attached),
                use_count,
            ),
        );
        Some(done)
    }

    /// `FishEventAddShape` (`G/char.cpp:11078-11121`): stamp the held shape at `pos` and
    /// answer its record, then check the board. A cell past 23, no held shape and a
    /// shape the tables never name end silently; a shape that does not fit re-sends the
    /// `BOX_USE` record and tells `That shape can not fit in this position`.
    fn fish_add_shape(&mut self, vid: Vid, pos: u8, mover: Mover) -> bool {
        if usize::from(pos) >= FISH_EVENT_SLOTS {
            return false;
        }
        let held = self.fish_board_mut(vid).attached;
        if held == 0 || held > FISH_EVENT_SHAPE_MAX {
            return false;
        }
        let fits = valid_position(&self.fish_board_mut(vid).cells, pos, held);
        if !fits {
            let (attached, use_count) = {
                let board = self.fish_board_mut(vid);
                (board.attached, board.use_count)
            };
            let _sent = self.write_to_client(
                vid,
                fish_event_record(
                    GcFishEventInfoSubheader::BoxUse,
                    u32::from(attached),
                    use_count,
                ),
            );
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"That shape can not fit in this position",
                &[],
            );
            return false;
        }
        place_shape(&mut self.fish_board_mut(vid).cells, pos, held);
        let _sent = self.write_to_client(
            vid,
            fish_event_record(
                GcFishEventInfoSubheader::ShapeAdd,
                u32::from(pos),
                u32::from(held),
            ),
        );
        true
    }

    /// `FishEventCheckEnd` (`G/char.cpp:11039-11076`): on a full board, give the reward
    /// and 30000 exp, clear the board and the held shape, and answer the `REWARD` record.
    /// Answers the rows the reward changed.
    fn finish_fish_shape(&mut self, vid: Vid, pid: u32, mover: Mover) -> Vec<RowChange> {
        if self
            .fish_board_mut(vid)
            .cells
            .iter()
            .any(|cell| cell.shape == 0)
        {
            return Vec::new();
        }
        let count = self.fish_board_mut(vid).use_count;
        let reward = reward_for(count);
        let questing = self.quest_running(vid);
        let mut changes = Vec::new();
        if let Some((_, given)) = self.auto_give(vid, (reward, 1), -1, questing) {
            let packed = MovedItems::new(pid, vid.raw(), given, mover, &self.protos, &self.locale);
            for record in packed.records {
                let _sent = self.write_to_client(vid, record);
            }
            changes.extend(packed.changes);
        }
        self.gain_pc_exp(vid, COMPLETION_EXP);
        let board = self.fish_board_mut(vid);
        for cell in &mut board.cells {
            *cell = FishCell::default();
        }
        board.use_count = 0;
        board.attached = 0;
        let _sent = self.write_to_client(
            vid,
            fish_event_record(GcFishEventInfoSubheader::Reward, reward, 0),
        );
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::{draw_shape, place_shape, reward_for, valid_position, FishCell, FISH_EVENT_SLOTS};
    use world::character::Dice;

    /// A [`Dice`] that answers one fixed draw.
    struct Fixed(u32);

    impl Dice for Fixed {
        fn random31(&mut self) -> u32 {
            self.0
        }
    }

    fn empty() -> [FishCell; FISH_EVENT_SLOTS] {
        [FishCell::default(); FISH_EVENT_SLOTS]
    }

    #[test]
    fn every_shape_lands_on_an_empty_board_at_its_first_head() {
        for shape in 1..=7u8 {
            assert!(valid_position(&empty(), 0, shape), "shape {shape}");
        }
    }

    #[test]
    fn no_shape_lands_past_its_last_head() {
        let board = empty();
        // The head counts of `FishEventIsValidPosition`'s table.
        for (shape, last) in [
            (1u8, 11u8),
            (2, 23),
            (3, 16),
            (4, 16),
            (5, 16),
            (6, 15),
            (7, 15),
        ] {
            assert!(valid_position(&board, last, shape), "shape {shape}");
            assert!(
                !valid_position(&board, last + 1, shape),
                "shape {shape} past {last}"
            );
        }
    }

    #[test]
    fn an_unknown_shape_lands_nowhere() {
        let board = empty();
        for shape in [0u8, 8, 255] {
            assert!(!valid_position(&board, 0, shape), "shape {shape}");
        }
    }

    #[test]
    fn a_covered_offset_refuses_the_head() {
        let mut board = empty();
        board[6].shape = 2;
        // Shape 1 at 0 checks 0, 6 and 12; cell 6 is taken.
        assert!(!valid_position(&board, 0, 1));
        assert!(valid_position(&board, 1, 1));
        // Shape 2 at 6 checks only 6.
        assert!(!valid_position(&board, 6, 2));
        assert!(valid_position(&board, 7, 2));
    }

    #[test]
    fn shape_7_checks_its_lowest_offset() {
        let mut board = empty();
        board[8].shape = 2;
        // Shape 7 at 0 checks 0, 1, 2, 6, 7 and 8; cell 8 is taken.
        assert!(!valid_position(&board, 0, 7));
        // At 3 it checks 3, 4, 5, 9, 10 and 11, all clear.
        assert!(valid_position(&board, 3, 7));
    }

    #[test]
    fn placing_a_shape_stamps_its_cells_with_one_main_cell() {
        let mut board = empty();
        place_shape(&mut board, 0, 1);
        for cell in [0, 6, 12] {
            assert_eq!(board[cell].shape, 1, "cell {cell}");
        }
        assert!(board[0].is_main);
        assert!(!board[6].is_main);
        assert!(!board[12].is_main);
        assert_eq!(board[1].shape, 0);

        let mut board = empty();
        place_shape(&mut board, 10, 7);
        for cell in [10, 11, 12, 16, 17, 18] {
            assert_eq!(board[cell].shape, 7, "cell {cell}");
        }
        assert!(board[10].is_main);
        assert!(!board[11].is_main);

        // Shape 2 stamps one cell, which is its main cell.
        let mut board = empty();
        place_shape(&mut board, 23, 2);
        assert_eq!(
            board[23],
            FishCell {
                is_main: true,
                shape: 2
            }
        );
    }

    #[test]
    fn an_unknown_shape_stamps_nothing() {
        let mut board = empty();
        place_shape(&mut board, 0, 9);
        assert!(board.iter().all(|cell| cell.shape == 0));
    }

    #[test]
    fn the_completion_reward_follows_the_use_count() {
        assert_eq!(reward_for(0), 25113);
        assert_eq!(reward_for(10), 25113);
        assert_eq!(reward_for(11), 25112);
        assert_eq!(reward_for(24), 25112);
        assert_eq!(reward_for(25), 25111);
        assert_eq!(reward_for(u32::MAX), 25111);
    }

    #[test]
    fn the_draw_is_1_to_6() {
        // Draw `d` answers `d % 6 + 1`; 5 draws 6, which a 1-5 range never could.
        for (draw, shape) in [(0u32, 1u8), (4, 5), (5, 6), (6, 1), (11, 6)] {
            assert_eq!(draw_shape(&mut Fixed(draw)), shape, "draw {draw}");
        }
    }
}
