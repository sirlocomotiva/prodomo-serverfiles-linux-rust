//! The dragon soul refine window and the two extracts: `CG_DRAGON_SOUL_REFINE`
//! (`CInputMain::Analyze`, `G/input_main.cpp:3885-3919`), which opens, closes and
//! refines the alchemy the table reader feeds (`DSManager`, `G/DragonSoul.cpp:599-1155`),
//! and the `ITEM_EXTRACT` landings (`DSManager::ExtractDragonHeart` and `PullOut`,
//! `G/DragonSoul.cpp:409-589`), which an extractor used on a stone runs.
//!
//! The request runs with no observer gate (`:3885` sits outside the guarded band) and
//! no waiting-quest guard. The six handled sub-headers are 0 open, 1 close, 2 grade, 3
//! step, 4 strength and 12 refine-all; the results 5..=11 share the enum and are
//! silently discarded on input, as legacy's missing `default:` does.
//!
//! The window is `ds_windows`: the opener's identity never matters, only that one
//! stands (`DragonSoul_RefineWindow_CanRefine`, `G/char_dragonsoul.cpp:165-168`). The
//! open answers the 5-byte record with sub 0 and the reserved position (ledger 281);
//! the close answers nothing, and the departures forget the window silently with the
//! rest. Death keeps it: no death path closes it.
//!
//! The refines gather their materials as the distinct stones of the grid in grid order,
//! so a duplicated cell counts once and an empty cell counts never, as legacy's
//! `std::set<LPITEM>` does. The grade reads its row by the first stone's type and
//! grade, the step by type and step; both refuse the empty grid with
//! `NOT_ENOUGH_MATERIAL`, a wrong-kind stone with `INVALID_MATERIAL`, a wrong count
//! with the under/over line, and short gold with `NOT_ENOUGH_MONEY`. The strength
//! takes exactly one soul and one refine material, refuses a second of either with
//! `TOO_MUCH_MATERIAL`, and refuses a stone no weight row lifts with `MAX_REFINE`.
//! The result vnum is `type * 10000 + grade * 1000 + step * 100 + refine * 11`
//! (`__FIX_INFO_REFINE_DRAGONSOUL__` is defined). The materials spend by count, the
//! result is auto-given, and the result record names its cell. The two extracts run as
//! landings of an extractor's use on a stone (`ITEM_EXTRACT`,
//! `G/char_item.cpp:7111-7141`): the heart extractor melts an unworn stone into a
//! charged heart by the grade's gamble over the charges (`ExtractDragonHeart`,
//! `G/DragonSoul.cpp:409-489`), and the soul extractor pulls a worn stone out of its
//! deck cell past the grade's draw, or breaks it into its by-product (`PullOut`,
//! `G/DragonSoul.cpp:491-589`). A heart or by-product that would not fit is laid at
//! the caller's feet, as `AutoGiveItem` lays it.
//!
//! New stones roll their attributes like every other creation (`PutAttributes`,
//! `:302-381`): the basics land scaled by the weight, then up to three distinct
//! additionals by draw. The strength instead copies the old stone's attributes and
//! rescales them (`RefreshItemAttributes`, `:150-216`). The refine-all sweeps one
//! grade box of one slot, 32 cells, refining each full set in turn.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::vid::Vid;
use gamedata::item_kind::{ITEM_DS, ITEM_MATERIAL};
use protocol::cg_dragon_soul::{
    CG_DRAGON_SOUL_GRID_SIZE, DS_SUB_HEADER_CLOSE, DS_SUB_HEADER_DO_REFINE_GRADE,
    DS_SUB_HEADER_DO_REFINE_STEP, DS_SUB_HEADER_DO_REFINE_STRENGTH, DS_SUB_HEADER_OPEN,
    DS_SUB_HEADER_REFINE_ALL, DS_SUB_HEADER_REFINE_FAIL,
    DS_SUB_HEADER_REFINE_FAIL_INVALID_MATERIAL, DS_SUB_HEADER_REFINE_FAIL_MAX_REFINE,
    DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MATERIAL, DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MONEY,
    DS_SUB_HEADER_REFINE_FAIL_TOO_MUCH_MATERIAL, DS_SUB_HEADER_REFINE_SUCCEED,
};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_vid::GcDragonSoulRefine;
use protocol::item_pos::ItemPos;
use world::character::{is_equip_position, MoveDone, MoveKind};
use world::item::ItemId;

use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{gold_delta, MoveItemRefused, MovedItems, Mover};

/// `MATERIAL_DS_REFINE_NORMAL..=HOLLY` (`C/item_length.h:333-335`).
const DS_MATERIAL_FIRST: i32 = 5;
/// `DRAGON_SOUL_GRADE_MAX` (`C/item_length.h:194`): the six grades, mythic last.
const DS_GRADES: u8 = 6;
/// `DRAGON_SOUL_BOX_SIZE` (`C/length.h:83`): the 32 cells of one grade box.
const DS_BOX: u16 = 32;
/// `DS_SLOT_MAX` (`C/item_length.h:181`): the six alchemy slots.
const DS_SLOTS: u8 = 6;

impl GameState {
    /// Whether the refine window of the character under `vid` stands:
    /// `DragonSoul_RefineWindow_CanRefine` (`G/char_dragonsoul.cpp:165-168`).
    pub(super) fn ds_window(&self, vid: Vid) -> bool {
        self.ds_windows.contains(&vid)
    }

    /// Run one `CG_DRAGON_SOUL_REFINE` for the character online under `vid`
    /// (`CInputMain::Analyze`, `G/input_main.cpp:3885-3919`).
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn dragon_soul_refine(
        &mut self,
        vid: Vid,
        sub_header: u8,
        grid: [ItemPos; CG_DRAGON_SOUL_GRID_SIZE],
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let mut moved = MovedItems::new(
            pid,
            vid.raw(),
            MoveDone {
                kind: MoveKind::Refined,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        match sub_header {
            DS_SUB_HEADER_OPEN => {
                self.ds_windows.insert(vid);
                let mut open = Vec::new();
                GcDragonSoulRefine {
                    sub_type: DS_SUB_HEADER_OPEN,
                    pos: ItemPos::new(1, u16::MAX),
                }
                .encode_into(&mut open);
                moved.records.push(open);
            }
            DS_SUB_HEADER_CLOSE => {
                self.ds_windows.remove(&vid);
            }
            DS_SUB_HEADER_DO_REFINE_GRADE => self.refine_ds_grade(vid, &grid, actor, &mut moved),
            DS_SUB_HEADER_DO_REFINE_STEP => self.refine_ds_step(vid, &grid, actor, &mut moved),
            DS_SUB_HEADER_DO_REFINE_STRENGTH => {
                self.refine_ds_strength(vid, &grid, actor, &mut moved);
            }
            DS_SUB_HEADER_REFINE_ALL => self.refine_ds_all(vid, &grid, actor, &mut moved),
            _ => {}
        }
        Ok(moved)
    }

    /// The refine result record: `SendRefineResultPacket` (`G/DragonSoul.cpp:1150-1168`).
    /// An invalid position leaves the reserved one the open used.
    fn ds_result(vid: Vid, sub: u8, pos: ItemPos, moved: &mut MovedItems) {
        let pos = if world::character::is_valid_item_position(pos) {
            pos
        } else {
            ItemPos::new(1, u16::MAX)
        };
        let mut record = Vec::new();
        GcDragonSoulRefine { sub_type: sub, pos }.encode_into(&mut record);
        let _ = vid;
        moved.records.push(record);
    }

    /// The vnum's `(type, grade, step, strength)` (`GetDragonSoulInfo`, `:100-106`).
    fn ds_info(vnum: u32) -> (u8, u8, u8, u8) {
        (
            u8::try_from(vnum / 10_000).unwrap_or(u8::MAX),
            u8::try_from((vnum / 1000) % 10).unwrap_or(u8::MAX),
            u8::try_from((vnum / 100) % 10).unwrap_or(u8::MAX),
            u8::try_from((vnum / 10) % 10).unwrap_or(u8::MAX),
        )
    }

    /// `MakeDragonSoulVnum` (`G/DragonSoul.cpp:394-401`) with
    /// `__FIX_INFO_REFINE_DRAGONSOUL__` on: the strength rides `* 11`, so the last
    /// digit is the stone's own number.
    fn ds_vnum(ds_type: u8, grade: u8, step: u8, strength: u8) -> u32 {
        u32::from(ds_type) * 10_000
            + u32::from(grade) * 1_000
            + u32::from(step) * 100
            + u32::from(strength) * 11
    }

    /// Whether the item `id` of the character under `vid` is a dragon soul stone:
    /// `CItem::IsDragonSoul` (`G/item.cpp:2786-2789`).
    fn is_ds_stone(&self, vid: Vid, id: ItemId) -> bool {
        self.characters.find_by_vid(vid).is_ok_and(|character| {
            character
                .items()
                .item(id)
                .and_then(|item| self.protos.get(item.vnum))
                .is_some_and(|proto| proto.item_type == ITEM_DS)
        })
    }

    /// The distinct items the grid names, in grid order, skipping the worn cells:
    /// legacy's `std::set<LPITEM>` gather (`:678-700`).
    fn ds_gather(&self, vid: Vid, grid: &[ItemPos; CG_DRAGON_SOUL_GRID_SIZE]) -> Vec<ItemId> {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        for cell in grid {
            if is_equip_position(*cell) {
                continue;
            }
            let Some(item) = character.items().item_at(*cell) else {
                continue;
            };
            if !ids.contains(&item.id) {
                ids.push(item.id);
            }
        }
        ids
    }

    /// The window gate of the three refines and the refine-all: without a window the
    /// grade chats its system error and the rest log theirs (`:666-673`, `:831-837`,
    /// `:990-996`, `:619-626`).
    fn ds_gated(&self, vid: Vid, actor: Mover, grade: bool) -> bool {
        if self.ds_window(vid) {
            return true;
        }
        if grade {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"[SYSTEM ERROR]You cannot upgrade dragon soul without refine window",
                &[] as &[Arg<'_>],
            );
        }
        false
    }

    /// Spend `need` of the stones' stacks in grid order (`:773-783`): whole stacks
    /// leave, the last stack keeps its rest, each with its record and its saved row.
    /// The counts are already known to cover `need`, so the spend always lands.
    fn ds_spend(
        &mut self,
        vid: Vid,
        actor: Mover,
        ids: &[ItemId],
        need: usize,
        moved: &mut MovedItems,
    ) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let mut plan = Vec::new();
        let mut left = need;
        for id in ids {
            if left == 0 {
                break;
            }
            let Some(item) = character.items().item(*id) else {
                continue;
            };
            let stack = usize::from(item.count);
            if left >= stack {
                plan.push((*id, None));
                left -= stack;
            } else {
                plan.push((*id, Some(stack - left)));
                left = 0;
            }
        }
        if left > 0 {
            return;
        }
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let mut done = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        for (id, rest) in plan {
            match rest {
                None => {
                    let Some(item) = items.item(id).cloned() else {
                        continue;
                    };
                    let pos = item.pos;
                    if items.release(id).is_ok() {
                        done.records.push(world::character::MoveRecord::Item(
                            world::character::ItemRecord::Set(world::item::gc_item_clear(pos)),
                        ));
                        done.changes
                            .push(world::character::ItemChange::Destroyed { id });
                    }
                }
                Some(rest) => {
                    let rest = u32::try_from(rest).unwrap_or(u32::MAX);
                    if items.set_count(id, rest).is_err() {
                        continue;
                    }
                    if let Some(item) = items.item(id) {
                        let update = item.gc_item_update();
                        done.records.push(world::character::MoveRecord::Item(
                            world::character::ItemRecord::Update(update),
                        ));
                        done.changes.push(world::character::ItemChange::Count {
                            id,
                            count: update.count,
                        });
                    }
                }
            }
        }
        world::character::sync_quickslots(&mut done, slots, items);
        let quickslots = character.quickslots().clone();
        let points = character.points().cloned();
        let pid = moved.owner_id;
        let done = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        moved.records.extend(done.records);
        moved.changes.extend(done.changes);
        moved.points = points;
        moved.quickslots = Some(quickslots);
    }

    /// Spend `fee` gold (`PointChange(POINT_GOLD, -fee)`): the record and the stored
    /// delta. The caller checked the purse covers the fee.
    fn ds_fee(&mut self, vid: Vid, actor: Mover, fee: u64, moved: &mut MovedItems) {
        let held = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::gold);
        let left = held.saturating_sub(fee);
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            character.set_gold(left);
        }
        let mut done = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        done.records.push(world::character::MoveRecord::Gold {
            amount: 0,
            value: left,
        });
        let pid = moved.owner_id;
        let packed = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        moved.records.extend(packed.records);
        moved.gold = Some(gold_delta(held, left));
    }

    /// `Gamble` (`G/DragonSoul.cpp:16-31`): the first probability whose running sum
    /// reaches the draw over the width. `None` when no sum reaches it.
    pub(super) fn ds_gamble(&mut self, probs: &[f32]) -> Option<usize> {
        let range: f32 = probs.iter().sum();
        let rolled = super::dragon_soul::fnumber_ds(&mut self.dice, range);
        Self::ds_pick(probs, rolled)
    }

    /// The gamble's pick for `rolled`: the first probability whose running sum reaches
    /// it. Split out so the boundary is testable without the dice.
    fn ds_pick(probs: &[f32], rolled: f32) -> Option<usize> {
        let mut sum = 0.0;
        for (index, prob) in probs.iter().enumerate() {
            sum += *prob;
            if sum >= rolled {
                return Some(index);
            }
        }
        None
    }

    /// `DSManager::DoRefineGrade` (`G/DragonSoul.cpp:656-815`): the distinct stones of
    /// the grid refine one grade row up or down the table, by the first stone's type
    /// and grade. A worn cell anywhere refuses silently; a non-soul stone chats the
    /// grade line and fails the stone.
    fn refine_ds_grade(
        &mut self,
        vid: Vid,
        grid: &[ItemPos; CG_DRAGON_SOUL_GRID_SIZE],
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        if !self.ds_gated(vid, actor, true) {
            return;
        }
        if grid.iter().any(|cell| is_equip_position(*cell)) {
            return;
        }
        let ids = self.ds_gather(vid, grid);
        if !self.ds_all_souls(vid, actor, &ids, moved, true) {
            return;
        }
        if ids.is_empty() {
            let pos = ItemPos::new(1, u16::MAX);
            Self::ds_result(
                vid,
                DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MATERIAL,
                pos,
                moved,
            );
            return;
        }
        let vnums = self.ds_vnums(vid, &ids);
        let Some(first) = vnums.first().copied() else {
            return;
        };
        let (ds_type, grade, _, _) = Self::ds_info(first);
        let Some((need, fee, probs)) = self
            .dragon_soul_table
            .refine_grade(grade)
            .map(|(n, f, p)| (n, f, p.to_vec()))
        else {
            let pos = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.items().item(ids[0]))
                .map_or(ItemPos::new(1, u16::MAX), |item| item.pos);
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"It is a dragon soul stone that cannot be upgraded.",
                &[] as &[Arg<'_>],
            );
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_INVALID_MATERIAL, pos, moved);
            return;
        };
        let pairs: Vec<(ItemId, u32)> = ids.iter().copied().zip(vnums).collect();
        if !self.ds_same_kind(vid, actor, &pairs, (ds_type, grade, 0), moved, true) {
            return;
        }
        let count = ids.len();
        if !Self::ds_count_ok(vid, count, need, moved) {
            return;
        }
        let held = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::gold);
        if held < u64::try_from(fee.max(0)).unwrap_or(u64::MAX) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"There is not enough money to make improvements.",
                &[] as &[Arg<'_>],
            );
            let pos = ItemPos::new(1, u16::MAX);
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MONEY, pos, moved);
            return;
        }
        let Some(result) = self.ds_gamble(&probs) else {
            return;
        };
        let vnum = Self::ds_vnum(ds_type, u8::try_from(result).unwrap_or(u8::MAX), 0, 0);
        if self.protos.get(vnum).is_none() {
            return;
        }
        self.ds_fee(vid, actor, u64::try_from(fee.max(0)).unwrap_or(0), moved);
        self.ds_spend(vid, actor, &ids, count, moved);
        let pos = self.ds_give(vid, actor, vnum, moved);
        if u8::try_from(result).unwrap_or(u8::MAX) > grade {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Grade improvement was successful.",
                &[] as &[Arg<'_>],
            );
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_SUCCEED, pos, moved);
        } else {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Grade improvement failed.",
                &[] as &[Arg<'_>],
            );
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL, pos, moved);
        }
    }

    /// `CreateItem` for a refine result of `vnum` (`G/item_manager.cpp:443-446`): the
    /// id, the drawn attributes and the duration socket
    /// (`DragonSoulItemInitialize`, `G/DragonSoul.cpp:383-392`). `None` when the ids
    /// are used up or the proto is gone.
    fn ds_make(&mut self, vnum: u32) -> Option<world::item::Item> {
        let (item_type, duration) = {
            let proto = self.protos.get(vnum)?;
            (proto.item_type, proto.values.first().copied().unwrap_or(0))
        };
        let id = self.item_ids.as_mut()?.allocate().ok()?;
        let mut stone = world::item::Item::new(id, vnum);
        stone.count = 1;
        let (ds_type, grade, _, _) = Self::ds_info(vnum);
        let table = self.dragon_soul_table.clone();
        let (Some(plain), Some(additionals), Some((basis, add_min, add_max)), Some(weight)) = (
            table.basic_applies(ds_type).map(<[_]>::to_vec),
            table.additional_applies(ds_type).map(<[_]>::to_vec),
            table.apply_numbers(grade),
            table.weight(
                grade,
                u8::try_from((vnum / 100) % 10).unwrap_or(u8::MAX),
                u8::try_from((vnum / 10) % 10).unwrap_or(u8::MAX),
            ),
        ) else {
            return Some(stone);
        };
        let factor = weight / 100.0;
        let take = usize::try_from(basis.max(0)).unwrap_or(0).min(plain.len());
        for (index, apply) in plain.iter().enumerate().take(take) {
            stone.attributes[index] = protocol::gc_item_window::ItemAttribute::new(
                u8::try_from(apply.apply_type).unwrap_or(u8::MAX),
                super::dragon_soul::weighed(apply.apply_value, factor),
            );
        }
        let draws = world::character::number(&mut self.dice, add_min, add_max).min(3);
        if draws > 0 {
            let probs: Vec<f32> = additionals.iter().map(|apply| apply.prob).collect();
            let count = usize::try_from(draws).unwrap_or(0);
            if let Some(picked) = self.distinct_draws_ds(&probs, count) {
                for (slot, which) in picked.iter().enumerate() {
                    if let Some(apply) = additionals.get(*which) {
                        stone.attributes[3 + slot] = protocol::gc_item_window::ItemAttribute::new(
                            u8::try_from(apply.apply_type).unwrap_or(u8::MAX),
                            super::dragon_soul::weighed(apply.apply_value, factor),
                        );
                    }
                }
            }
        }
        if item_type == ITEM_DS && duration > 0 {
            let _ = stone.set_socket(super::dragon_soul::REMAIN_SOCKET, duration);
        }
        Some(stone)
    }

    /// Make the melted heart (`CreateItem(DRAGON_HEART_VNUM)`, `G/DragonSoul.cpp:468`):
    /// one heart of `charge` percent. The creation stamps the expiry socket, as
    /// `CreateItem` does for a `LIMIT_REAL_TIME` proto (`G/item_manager.cpp:279-294`):
    /// the owner's heart carries `REAL_TIME` 86400, so an unstamped heart's socket 0
    /// reads 0 and the expiry pulse removes it within the second. `None` when no
    /// proto, no ids, or the charge is zero — the fail arm, which spends the pair
    /// and melts nothing.
    pub(super) fn ds_make_heart(&mut self, charge: i32) -> Option<world::item::Item> {
        if charge <= 0 {
            return None;
        }
        let proto = self.protos.get(super::dragon_soul::HEART_VNUM)?.clone();
        let id = self.item_ids.as_mut()?.allocate().ok()?;
        let mut heart = world::item::Item::new(id, super::dragon_soul::HEART_VNUM);
        heart.count = 1;
        heart.size = {
            let [size, ..] = proto.size.to_le_bytes();
            size
        };
        heart.flags = proto.flags;
        heart.anti_flags = proto.anti_flags;
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            });
        world::character::stamp_created_sockets(&mut heart, &proto, now_secs);
        Some(heart)
    }

    /// `MakeDistinctRandomNumberSet` through the game dice, for the made stone's
    /// additional attributes.
    fn distinct_draws_ds(&mut self, probs: &[f32], count: usize) -> Option<Vec<usize>> {
        if probs.len() < count {
            return None;
        }
        let mut left: Vec<(usize, f32)> = probs.iter().copied().enumerate().collect();
        let mut picked = Vec::with_capacity(count);
        for _ in 0..count {
            let range: f32 = left.iter().map(|(_, prob)| *prob).sum();
            let rolled = super::dragon_soul::fnumber_ds(&mut self.dice, range);
            let mut sum = 0.0;
            let mut won = None;
            for (slot, (index, prob)) in left.iter().enumerate() {
                sum += *prob;
                if sum >= rolled {
                    won = Some((slot, *index));
                    break;
                }
            }
            let (slot, index) = won?;
            left.remove(slot);
            picked.push(index);
        }
        Some(picked)
    }

    /// Make the refine result and auto-give it, answering where it landed:
    /// `CreateItem` (`G/item_manager.cpp:443-446`) then `AutoGiveItem`
    /// (`G/char_item.cpp:9027-9075`). The made stone carries its drawn attributes
    /// (`DragonSoulItemInitialize`, `G/DragonSoul.cpp:383-392`); an unplaced stone lies
    /// at the caller's feet, and the result record names the reserved position.
    fn ds_give(&mut self, vid: Vid, actor: Mover, vnum: u32, moved: &mut MovedItems) -> ItemPos {
        let failed = ItemPos::new(1, u16::MAX);
        let Some(stone) = self.ds_make(vnum) else {
            return failed;
        };
        let questing = self.quest_running(vid);
        let Some((given, done)) = self.auto_give_made(vid, stone, questing) else {
            return failed;
        };
        let pos = match given {
            world::character::AutoGiven::Placed { pos, .. } => pos,
            world::character::AutoGiven::Merged(id) => self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.items().item(id))
                .map_or(failed, |item| item.pos),
            world::character::AutoGiven::Unplaced(item) => {
                self.lay_auto_given(vid, item);
                failed
            }
            world::character::AutoGiven::NoProto | world::character::AutoGiven::NotMade => failed,
        };
        let pid = moved.owner_id;
        let packed = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        moved.records.extend(packed.records);
        moved.changes.extend(packed.changes);
        pos
    }

    /// `DSManager::DoRefineStep` (`G/DragonSoul.cpp:817-976`): the step twin of the
    /// grade — the row by type and step, the same count and gold gates, and a result
    /// that keeps the grade and rolls the step.
    fn refine_ds_step(
        &mut self,
        vid: Vid,
        grid: &[ItemPos; CG_DRAGON_SOUL_GRID_SIZE],
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        if !self.ds_gated(vid, actor, false) {
            return;
        }
        let ids = self.ds_gather(vid, grid);
        if !self.ds_all_souls(vid, actor, &ids, moved, false) {
            return;
        }
        if ids.is_empty() {
            let pos = ItemPos::new(1, u16::MAX);
            Self::ds_result(
                vid,
                DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MATERIAL,
                pos,
                moved,
            );
            return;
        }
        let vnums = self.ds_vnums(vid, &ids);
        let Some(first) = vnums.first().copied() else {
            return;
        };
        let (ds_type, grade, step, _) = Self::ds_info(first);
        let Some((need, fee, probs)) = self
            .dragon_soul_table
            .refine_step(step)
            .map(|(n, f, p)| (n, f, p.to_vec()))
        else {
            let pos = self.ds_pos_of(vid, ids[0]);
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"It is a dragon soul stone that cannot be upgraded.",
                &[] as &[Arg<'_>],
            );
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_INVALID_MATERIAL, pos, moved);
            return;
        };
        let pairs: Vec<(ItemId, u32)> = ids.iter().copied().zip(vnums).collect();
        if !self.ds_same_kind(vid, actor, &pairs, (ds_type, grade, step), moved, false) {
            return;
        }
        let count = ids.len();
        if !Self::ds_count_ok(vid, count, need, moved) {
            return;
        }
        let held = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::gold);
        if held < u64::try_from(fee.max(0)).unwrap_or(u64::MAX) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"There is not enough money to make improvements.",
                &[] as &[Arg<'_>],
            );
            let pos = ItemPos::new(1, u16::MAX);
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MONEY, pos, moved);
            return;
        }
        let Some(result) = self.ds_gamble(&probs) else {
            return;
        };
        let vnum = Self::ds_vnum(ds_type, grade, u8::try_from(result).unwrap_or(u8::MAX), 0);
        if self.protos.get(vnum).is_none() {
            return;
        }
        self.ds_fee(vid, actor, u64::try_from(fee.max(0)).unwrap_or(0), moved);
        self.ds_spend(vid, actor, &ids, count, moved);
        let pos = self.ds_give(vid, actor, vnum, moved);
        if u8::try_from(result).unwrap_or(u8::MAX) > step {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Step improvement was successful.",
                &[] as &[Arg<'_>],
            );
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_SUCCEED, pos, moved);
        } else {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Step improvement failed.",
                &[] as &[Arg<'_>],
            );
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL, pos, moved);
        }
    }

    /// Every gathered stone a dragon soul (`:688-704`): the grade line or the level
    /// line names the kind, and the failing stone's cell names the record. Answers
    /// whether all passed.
    fn ds_all_souls(
        &mut self,
        vid: Vid,
        actor: Mover,
        ids: &[ItemId],
        moved: &mut MovedItems,
        grade: bool,
    ) -> bool {
        for id in ids {
            if self.is_ds_stone(vid, *id) {
                continue;
            }
            let pos = self.ds_pos_of(vid, *id);
            let line = if grade {
                b"It is not a material required for grade improvement."
            } else {
                b"It is not a material required for level improvement."
            };
            self.tell(vid, actor, CHAT_TYPE_INFO, line, &[] as &[Arg<'_>]);
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_INVALID_MATERIAL, pos, moved);
            return false;
        }
        true
    }

    /// Every stone the first's kind (`:707-723`): the type and grade always, the step
    /// too for the level run (`grade` false names the level line). The failing stone's
    /// cell names the record. Answers whether all matched.
    fn ds_same_kind(
        &mut self,
        vid: Vid,
        actor: Mover,
        stones: &[(ItemId, u32)],
        first: (u8, u8, u8),
        moved: &mut MovedItems,
        grade_run: bool,
    ) -> bool {
        let (ds_type, grade, step) = first;
        for (id, vnum) in stones {
            let (kind, level, rung, _) = Self::ds_info(*vnum);
            let same = if grade_run {
                kind == ds_type && level == grade
            } else {
                kind == ds_type && level == grade && rung == step
            };
            if same {
                continue;
            }
            let pos = self.ds_pos_of(vid, *id);
            let line = if grade_run {
                b"It is not a material required for grade improvement."
            } else {
                b"It is not a material required for level improvement."
            };
            self.tell(vid, actor, CHAT_TYPE_INFO, line, &[] as &[Arg<'_>]);
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_INVALID_MATERIAL, pos, moved);
            return false;
        }
        true
    }

    /// The gathered count answers the row's (`:724-740`): too few chats nothing and
    /// fails the reserved position, too many fails it too. Answers whether it matched.
    fn ds_count_ok(vid: Vid, count: usize, need: i32, moved: &mut MovedItems) -> bool {
        let need = usize::try_from(need.max(0)).unwrap_or(usize::MAX);
        if count == need {
            return true;
        }
        let sub = if count < need {
            DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MATERIAL
        } else {
            DS_SUB_HEADER_REFINE_FAIL_TOO_MUCH_MATERIAL
        };
        Self::ds_result(vid, sub, ItemPos::new(1, u16::MAX), moved);
        false
    }

    /// The cell of `id`'s stone, or the reserved position when it is gone.
    fn ds_pos_of(&self, vid: Vid, id: ItemId) -> ItemPos {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item(id))
            .map_or(ItemPos::new(1, u16::MAX), |item| item.pos)
    }

    /// The vnums of `ids`' stones, in order, skipping the gone.
    fn ds_vnums(&self, vid: Vid, ids: &[ItemId]) -> Vec<u32> {
        ids.iter()
            .filter_map(|id| {
                self.characters
                    .find_by_vid(vid)
                    .ok()
                    .and_then(|character| character.items().item(*id))
                    .map(|item| item.vnum)
            })
            .collect()
    }

    /// `DSManager::DoRefineStrength` (`G/DragonSoul.cpp:978-1148`): one soul and one
    /// refine material harden one strength up on a win, or soften one down on a
    /// loss — unless the stone stands at 0, and then nothing is given. The result
    /// copies the old attributes and rescales them (`RefreshItemAttributes`,
    /// `:150-216`).
    fn refine_ds_strength(
        &mut self,
        vid: Vid,
        grid: &[ItemPos; CG_DRAGON_SOUL_GRID_SIZE],
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        if !self.ds_gated(vid, actor, false) {
            return;
        }
        let ids = self.ds_gather(vid, grid);
        if ids.is_empty() {
            return;
        }
        let Some((soul, material)) = self.ds_pair(vid, actor, &ids, moved) else {
            return;
        };
        let Some(stone) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item(soul).cloned())
        else {
            return;
        };
        let (_, grade, step, strength) = Self::ds_info(stone.vnum);
        let weight = self
            .dragon_soul_table
            .weight(grade, step, strength.saturating_add(1));
        if weight.is_none_or(|weight| weight < f32::EPSILON) {
            let pos = self.ds_pos_of(vid, soul);
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"It is a dragon soul stone that cannot be enhanced.",
                &[] as &[Arg<'_>],
            );
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_MAX_REFINE, pos, moved);
            return;
        }
        let material_sub = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item(material))
            .and_then(|item| self.protos.get(item.vnum))
            .map_or(u8::MAX, |proto| {
                u8::try_from(proto.sub_type).unwrap_or(u8::MAX)
            });
        let Some((fee, probs)) = self
            .dragon_soul_table
            .refine_strength(material_sub)
            .map(|(fee, probs)| (fee, probs.to_vec()))
        else {
            let pos = self.ds_pos_of(vid, soul);
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"It is a dragon soul stone that cannot be enhanced.",
                &[] as &[Arg<'_>],
            );
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_INVALID_MATERIAL, pos, moved);
            return;
        };
        let Some(prob) = probs.get(usize::from(strength)).copied() else {
            let pos = self.ds_pos_of(vid, soul);
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_INVALID_MATERIAL, pos, moved);
            return;
        };
        let held = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::gold);
        if held < u64::try_from(fee.max(0)).unwrap_or(u64::MAX) {
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"There is not enough money to make improvements.",
                &[] as &[Arg<'_>],
            );
            let pos = ItemPos::new(1, u16::MAX);
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MONEY, pos, moved);
            return;
        }
        self.ds_fee(vid, actor, u64::try_from(fee.max(0)).unwrap_or(0), moved);
        let rolled = super::dragon_soul::fnumber_ds(&mut self.dice, 100.0);
        let pair = (soul, material);
        let draws = (prob, rolled);
        self.ds_strength_outcome(vid, actor, &stone, pair, draws, moved);
    }

    /// The strength draw's three outcomes (`:1103-1150`): a win hardens one rung up,
    /// a loss at 0 spends the pair and gives nothing, a loss above 0 softens one rung
    /// down. The copies carry the old attributes through `RefreshItemAttributes`.
    fn ds_strength_outcome(
        &mut self,
        vid: Vid,
        actor: Mover,
        stone: &world::item::Item,
        pair: (ItemId, ItemId),
        draws: (f32, f32),
        moved: &mut MovedItems,
    ) {
        let (soul, material) = pair;
        let (prob, rolled) = draws;
        let (ds_type, grade, step, strength) = Self::ds_info(stone.vnum);
        if rolled <= prob {
            let vnum = Self::ds_vnum(ds_type, grade, step, strength.saturating_add(1));
            if self.protos.get(vnum).is_none() {
                return;
            }
            let attributes = stone.attributes;
            self.ds_spend(vid, actor, &[soul, material], 2, moved);
            let pos = self.ds_give(vid, actor, vnum, moved);
            self.ds_refresh(vid, actor, vnum, pos, attributes, moved);
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Reinforcement was successful.",
                &[] as &[Arg<'_>],
            );
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_SUCCEED, pos, moved);
        } else if strength == 0 {
            self.ds_spend(vid, actor, &[soul, material], 2, moved);
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Hardening failed.",
                &[] as &[Arg<'_>],
            );
            let pos = ItemPos::new(1, u16::MAX);
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL, pos, moved);
        } else {
            let vnum = Self::ds_vnum(ds_type, grade, step, strength - 1);
            if self.protos.get(vnum).is_none() {
                return;
            }
            let attributes = stone.attributes;
            self.ds_spend(vid, actor, &[soul, material], 2, moved);
            let pos = self.ds_give(vid, actor, vnum, moved);
            self.ds_refresh(vid, actor, vnum, pos, attributes, moved);
            self.tell(
                vid,
                actor,
                CHAT_TYPE_INFO,
                b"Hardening failed.",
                &[] as &[Arg<'_>],
            );
            Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL, pos, moved);
        }
    }

    /// One soul and one refine material (`:1018-1055`): a second of either fails too
    /// much, anything else chats the reinforcement line and fails it. Answers the pair,
    /// or `None` when the grid held no pair.
    fn ds_pair(
        &mut self,
        vid: Vid,
        actor: Mover,
        ids: &[ItemId],
        moved: &mut MovedItems,
    ) -> Option<(ItemId, ItemId)> {
        let mut soul = None;
        let mut material = None;
        for id in ids {
            let Some(proto) = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.items().item(*id))
                .and_then(|item| self.protos.get(item.vnum))
            else {
                continue;
            };
            if proto.item_type == ITEM_DS {
                if soul.is_some() {
                    let pos = self.ds_pos_of(vid, *id);
                    Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_TOO_MUCH_MATERIAL, pos, moved);
                    return None;
                }
                soul = Some(*id);
            } else if proto.item_type == ITEM_MATERIAL
                && (DS_MATERIAL_FIRST..DS_MATERIAL_FIRST + 3).contains(&proto.sub_type)
            {
                if material.is_some() {
                    let pos = self.ds_pos_of(vid, *id);
                    Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_TOO_MUCH_MATERIAL, pos, moved);
                    return None;
                }
                material = Some(*id);
            } else {
                let pos = self.ds_pos_of(vid, *id);
                self.tell(
                    vid,
                    actor,
                    CHAT_TYPE_INFO,
                    b"It is not a material required for reinforcement.",
                    &[] as &[Arg<'_>],
                );
                Self::ds_result(vid, DS_SUB_HEADER_REFINE_FAIL_INVALID_MATERIAL, pos, moved);
                return None;
            }
        }
        if let (Some(soul), Some(material)) = (soul, material) {
            return Some((soul, material));
        }
        Self::ds_result(
            vid,
            DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MATERIAL,
            ItemPos::new(1, u16::MAX),
            moved,
        );
        None
    }

    /// `RefreshItemAttributes` (`G/DragonSoul.cpp:150-216`) for the stone `auto_give`
    /// just placed: the old attributes copy over, then the basics land scaled by the
    /// new weight and the additionals rescale in place.
    fn ds_refresh(
        &mut self,
        vid: Vid,
        actor: Mover,
        vnum: u32,
        placed: ItemPos,
        attributes: [protocol::gc_item_window::ItemAttribute; 7],
        moved: &mut MovedItems,
    ) {
        let (ds_type, grade, step, strength) = Self::ds_info(vnum);
        let table = self.dragon_soul_table.clone();
        let (Some(plain), Some(additionals), Some(weight)) = (
            table.basic_applies(ds_type).map(<[_]>::to_vec),
            table.additional_applies(ds_type).map(<[_]>::to_vec),
            table.weight(grade, step, strength),
        ) else {
            return;
        };
        let factor = weight / 100.0;
        let mut pairs = Vec::new();
        // `CopyAttributeTo` first: the placed stone starts as the old stone's copy,
        // and the rescale below overwrites every written index.
        for (index, apply) in plain.iter().enumerate() {
            pairs.push((
                index,
                u8::try_from(apply.apply_type).unwrap_or(u8::MAX),
                super::dragon_soul::weighed(apply.apply_value, factor),
            ));
        }
        for (index, kept) in attributes.iter().enumerate().skip(3) {
            if kept.b_type == 0 {
                continue;
            }
            let value = additionals
                .iter()
                .find(|apply| u8::try_from(apply.apply_type).unwrap_or(u8::MAX) == kept.b_type)
                .map_or(0, |apply| apply.apply_value);
            pairs.push((
                index,
                kept.b_type,
                super::dragon_soul::weighed(value, factor),
            ));
        }
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let Some(stone) = character.items().item_at(placed) else {
            return;
        };
        let id = stone.id;
        let (items, _, _) = character.items_points_and_quickslots_mut();
        let mut done = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        for (index, b_type, s_value) in pairs {
            let forced =
                world::character::force_worn_attribute(items, id, (index, b_type, s_value));
            done.records.extend(forced.records);
            done.changes.extend(forced.changes);
        }
        let pid = moved.owner_id;
        let done = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        moved.records.extend(done.records);
        moved.changes.extend(done.changes);
    }

    /// `DSManager::DoAllRefineGrade` (`G/DragonSoul.cpp:599-654`): sweep one grade box
    /// of one slot — 32 soul-inventory cells — refining each full set in turn. The two
    /// words are the slot and the grade; past either, nothing runs.
    fn refine_ds_all(
        &mut self,
        vid: Vid,
        grid: &[ItemPos; CG_DRAGON_SOUL_GRID_SIZE],
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        if !self.ds_gated(vid, actor, false) {
            return;
        }
        let slot = grid[0].cell;
        let grade = grid[1].cell;
        if slot >= u16::from(DS_SLOTS) || grade >= u16::from(DS_GRADES) {
            return;
        }
        let start = u32::from(slot) * u32::from(DS_GRADES) * u32::from(DS_BOX)
            + u32::from(grade) * u32::from(DS_BOX);
        let mut cells = Vec::new();
        for offset in 0..u32::from(DS_BOX) {
            let cell = start + offset;
            let Ok(cell) = u16::try_from(cell) else {
                continue;
            };
            let pos = ItemPos::new(5, cell);
            let Ok(character) = self.characters.find_by_vid(vid) else {
                break;
            };
            if character.items().item_at(pos).is_some() {
                cells.push(pos);
            }
        }
        let mut at = 0;
        while at < cells.len() {
            let mut sweep = [ItemPos::new(1, u16::MAX); CG_DRAGON_SOUL_GRID_SIZE];
            for (slot, cell) in cells[at..]
                .iter()
                .take(CG_DRAGON_SOUL_GRID_SIZE)
                .enumerate()
            {
                sweep[slot] = *cell;
            }
            let before = moved.records.len();
            self.refine_ds_grade(vid, &sweep, actor, moved);
            if moved.records.len() == before {
                break;
            }
            at += CG_DRAGON_SOUL_GRID_SIZE.min(cells.len() - at);
        }
    }
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use common::vid::Vid;
    use protocol::item_pos::ItemPos;
    use tokio::sync::mpsc::UnboundedReceiver;
    use world::item::Item;

    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};
    use crate::game_state::Pcg32;

    /// The owner's first soul stone: vnum 110000, type 11, grade/step/strength 0.
    const STONE: u32 = 110_000;

    /// A stone of `id` and `vnum`, with `count` and no attributes.
    fn a_stone(id: u32, vnum: u32, count: u16) -> Item {
        let mut stone = Item::new(id, vnum);
        stone.count = count;
        stone
    }

    /// A world with the owner's soul table loaded, one entered player holding `items`
    /// and `gold`, its inbox drained of the enter.
    fn entered_with_table(
        items: &[(ItemPos, Item)],
        gold: u64,
    ) -> (GameState, UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        let table = gamedata::dragon_soul_table::DragonSoulTable::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../legacy/gamedata/locale/europe"),
        )
        .expect("the owner's soul table loads");
        let mut world = a_world(&clock).with_dragon_soul_table(std::sync::Arc::new(table));
        world
            .install_item_ids(world::item::ItemIdRange::new(1, 1_000_000, 1).expect("a test range"))
            .expect("the first install");
        let (mut inbox, _) = enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, items);
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("the player is in the world")
            .set_gold(gold);
        while inbox.try_recv().is_ok() {}
        (world, inbox)
    }

    /// A grid naming `cells` of the soul inventory, the rest the reserved position.
    fn a_grid(cells: &[u16]) -> [ItemPos; CG_DRAGON_SOUL_GRID_SIZE] {
        let mut grid = [ItemPos::new(1, u16::MAX); CG_DRAGON_SOUL_GRID_SIZE];
        for (slot, cell) in cells.iter().enumerate() {
            grid[slot] = ItemPos::new(EWindows::DragonSoulInventory as u8, *cell);
        }
        grid
    }

    /// The result sub-headers of `moved`'s records, in order.
    fn results(moved: &MovedItems) -> Vec<u8> {
        moved
            .records
            .iter()
            .filter(|frame| {
                frame.first() == Some(&protocol::gc_inventory::HEADER_GC_DRAGON_SOUL_REFINE.value())
            })
            .map(|frame| frame[1])
            .collect()
    }

    /// Fifteen grade-0 stones refine: the fee leaves, the grid empties, a stone lands,
    /// and the result names success or fail by the draw; too few stones fail the count
    /// with nothing spent.
    #[test]
    fn the_grade_refine_spends_and_lands_and_too_few_fail_the_count() {
        let vid = Vid::new(1);
        let mut items = Vec::new();
        for cell in 0..15u16 {
            items.push((
                ItemPos::new(EWindows::DragonSoulInventory as u8, cell),
                a_stone(100 + u32::from(cell), STONE, 1),
            ));
        }
        let (mut world, mut inbox) = entered_with_table(&items, 1_000_000);
        world.dice = Pcg32::new(0, 0);
        let actor = world.own_mover(vid).expect("the entrant moves");
        world
            .dragon_soul_refine(vid, DS_SUB_HEADER_OPEN, a_grid(&[]), actor)
            .expect("the window opens");
        let character = world.characters.find_by_vid(vid).expect("the player is in");
        let here: Vec<u16> = (0..15u16)
            .filter(|cell| {
                character
                    .items()
                    .item_at(ItemPos::new(EWindows::DragonSoulInventory as u8, *cell))
                    .is_some()
            })
            .collect();
        assert_eq!(here.len(), 15, "all fifteen stones land: {here:?}");
        let proto = world.protos.get(STONE);
        assert!(
            proto.is_some_and(|proto| proto.item_type == ITEM_DS),
            "110000 is ITEM_DS"
        );
        assert!(world.protos.get(111_000).is_some(), "111000 has a proto");
        let cells: Vec<u16> = (0..15).collect();
        let moved = world
            .dragon_soul_refine(vid, DS_SUB_HEADER_DO_REFINE_GRADE, a_grid(&cells), actor)
            .expect("the refine runs");
        let heard: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        // Pcg32(0, 0)'s first draw is `0xe4c1_4788 >> 1` over `RAND_MAX` of 100: 89.4,
        // inside the grade-0 row's 70+25, so the gamble lands grade 1: a success result
        // with a grade-1 stone given.
        assert_eq!(
            results(&moved),
            vec![DS_SUB_HEADER_REFINE_SUCCEED],
            "grade 1 succeeds: {heard:02x?}"
        );
        let left = world
            .characters
            .find_by_vid(vid)
            .expect("the player stays")
            .gold();
        assert_eq!(left, 1_000_000 - 30_000, "the grade-0 fee leaves");
        assert_eq!(moved.gold, Some(-30_000), "the stored delta rides");
        let result_pos = moved
            .records
            .iter()
            .find(|frame| {
                frame.first() == Some(&protocol::gc_inventory::HEADER_GC_DRAGON_SOUL_REFINE.value())
            })
            .map(|frame| (frame[2], u16::from_le_bytes([frame[3], frame[4]])));
        let character = world.characters.find_by_vid(vid).expect("the player stays");
        let kept: Vec<(u16, u32)> = (0..15u16)
            .filter_map(|cell| {
                character
                    .items()
                    .item_at(ItemPos::new(EWindows::DragonSoulInventory as u8, cell))
                    .map(|item| (cell, item.vnum))
            })
            .collect();
        assert!(kept.is_empty(), "the grid empties: {kept:?}");
        assert_eq!(
            result_pos,
            Some((1, 0)),
            "the result names the inventory cell: {result_pos:?}"
        );
        let landed = character
            .items()
            .item_at(ItemPos::new(EWindows::Inventory as u8, 0))
            .map(|item| item.vnum);
        assert_eq!(
            landed,
            Some(111_000),
            "the grade-1 stone lands in the inventory"
        );
    }

    /// The gamble's boundary: a draw exactly on a running sum picks that index, not
    /// the next; past every sum nothing picks.
    #[test]
    fn the_gamble_pick_lands_on_its_boundary() {
        let row = [70.0, 25.0, 5.0];
        assert_eq!(GameState::ds_pick(&row, 70.0), Some(0));
        assert_eq!(GameState::ds_pick(&row, 95.0), Some(1));
        assert_eq!(GameState::ds_pick(&row, 100.0), Some(2));
        assert_eq!(GameState::ds_pick(&row, 100.1), None);
    }

    /// Too few stones fail the count: one stone, nothing spent, no result stone.
    #[test]
    fn too_few_stones_fail_the_count_with_nothing_spent() {
        let vid = Vid::new(1);
        let (mut short, _) = entered_with_table(
            &[(
                ItemPos::new(EWindows::DragonSoulInventory as u8, 0),
                a_stone(7, STONE, 1),
            )],
            1_000_000,
        );
        let actor = short.own_mover(vid).expect("the entrant moves");
        short
            .dragon_soul_refine(vid, DS_SUB_HEADER_OPEN, a_grid(&[]), actor)
            .expect("the window opens");
        let actor = short.own_mover(vid).expect("the entrant moves");
        let failed = short
            .dragon_soul_refine(vid, DS_SUB_HEADER_DO_REFINE_GRADE, a_grid(&[0]), actor)
            .expect("the refine runs");
        assert_eq!(
            results(&failed),
            vec![DS_SUB_HEADER_REFINE_FAIL_NOT_ENOUGH_MATERIAL],
            "too few fail the count"
        );
        assert_eq!(
            short
                .characters
                .find_by_vid(vid)
                .expect("the player stays")
                .gold(),
            1_000_000,
            "nothing spent"
        );
    }

    /// A world with the window open, holding a soul of `vnum` and one normal refine
    /// material, its dice seeded.
    fn entered_with_pair(vnum: u32) -> (GameState, Vid) {
        let vid = Vid::new(1);
        let soul_at = ItemPos::new(EWindows::DragonSoulInventory as u8, 0);
        let mat_at = ItemPos::new(EWindows::DragonSoulInventory as u8, 1);
        let (mut world, _) = entered_with_table(
            &[
                (soul_at, a_stone(7, vnum, 1)),
                (mat_at, {
                    let mut material = Item::new(8, 100_300);
                    material.count = 1;
                    material
                }),
            ],
            1_000_000,
        );
        let actor = world.own_mover(vid).expect("the entrant moves");
        world
            .dragon_soul_refine(vid, DS_SUB_HEADER_OPEN, a_grid(&[]), actor)
            .expect("the window opens");
        world.dice = Pcg32::new(0, 0);
        (world, vid)
    }

    /// Strength 0 with a normal material hardens to 1: prob 100, any draw wins, and
    /// the fee leaves.
    #[test]
    fn the_strength_win_hardens_one_rung_up() {
        let (mut world, vid) = entered_with_pair(STONE);
        let actor = world.own_mover(vid).expect("the entrant moves");
        let moved = world
            .dragon_soul_refine(
                vid,
                DS_SUB_HEADER_DO_REFINE_STRENGTH,
                a_grid(&[0, 1]),
                actor,
            )
            .expect("the refine runs");
        assert_eq!(
            results(&moved),
            vec![DS_SUB_HEADER_REFINE_SUCCEED],
            "prob 100 always wins"
        );
        let landed = world
            .characters
            .find_by_vid(vid)
            .expect("the player stays")
            .items()
            .item_at(ItemPos::new(1, 0))
            .map(|item| item.vnum);
        assert_eq!(landed, Some(110_011), "strength 1 lands in the inventory");
        assert_eq!(
            world
                .characters
                .find_by_vid(vid)
                .expect("the player stays")
                .gold(),
            1_000_000 - 10_000,
            "the normal fee leaves"
        );
    }

    /// Strength 1 with a normal material: Pcg32(0, 0)'s 89.4 beats prob 80, so the
    /// loss softens to 0 with a stone given.
    #[test]
    fn the_strength_loss_above_zero_softens_one_rung_down() {
        let (mut world, vid) = entered_with_pair(110_011);
        let actor = world.own_mover(vid).expect("the entrant moves");
        let moved = world
            .dragon_soul_refine(
                vid,
                DS_SUB_HEADER_DO_REFINE_STRENGTH,
                a_grid(&[0, 1]),
                actor,
            )
            .expect("the refine runs");
        assert_eq!(
            results(&moved),
            vec![DS_SUB_HEADER_REFINE_FAIL],
            "89.4 beats 80"
        );
        let landed = world
            .characters
            .find_by_vid(vid)
            .expect("the player stays")
            .items()
            .item_at(ItemPos::new(1, 0))
            .map(|item| item.vnum);
        assert_eq!(landed, Some(110_000), "the loss softens to strength 0");
    }

    /// Strength 2 with a normal material: weight(0, 0, 3) is 0, so the stone cannot
    /// be enhanced and nothing moves.
    #[test]
    fn past_the_weights_the_stone_cannot_be_enhanced() {
        let (mut world, vid) = entered_with_pair(110_022);
        let actor = world.own_mover(vid).expect("the entrant moves");
        let moved = world
            .dragon_soul_refine(
                vid,
                DS_SUB_HEADER_DO_REFINE_STRENGTH,
                a_grid(&[0, 1]),
                actor,
            )
            .expect("the refine runs");
        assert_eq!(
            results(&moved),
            vec![DS_SUB_HEADER_REFINE_FAIL_MAX_REFINE],
            "past the weights"
        );
        assert_eq!(
            world
                .characters
                .find_by_vid(vid)
                .expect("the player stays")
                .gold(),
            1_000_000,
            "nothing spent"
        );
    }
}
