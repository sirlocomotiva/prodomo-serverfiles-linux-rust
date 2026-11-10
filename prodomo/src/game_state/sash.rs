//! The sash window (`__SASH_SYSTEM__`): the `CG_SASH` request (`CInputMain::Sash`,
//! `G/input_main.cpp:3395-3425`), which adds, removes, refines and closes the two
//! materials `OpenSash` stood up (`G/char.cpp:9888-10560`).
//!
//! The request runs only while no quest waits on the character (`GetPCForce(...)->IsRunning()`),
//! with no observer gate (`G/input_main.cpp:3862` sits outside the guarded band). An unknown
//! sub-header is absorbed, as legacy's `default: break` does.
//!
//! The window is `sash_windows`: combination or absorption. Its materials are item ids, so a
//! held item stays held when it moves cells; legacy holds `LPITEM` pointers, which survive the
//! same moves. A close (`CloseSash`, `:9936-9965`) unlocks both and answers the close record;
//! without a window it sends nothing (`!m_bSashCombination && !m_bSashAbsorption` returns before
//! the send). Death closes it (`DeathPenalty`, `G/char_battle.cpp:330-336`, and `Dead`,
//! `:1620-1627`), a warp or disconnect forgets it silently with the refine state.
//!
//! The add (`AddSashMaterial`, `:10105-10213`) resolves a `pos` at or past 2 to the first free
//! slot only when it is exactly 255 — legacy's report is `CUSTOM_INVENTORY_SLOT_END` (1370
//! under the custom inventory), which never fits a `BYTE`, so the client always sends 255; the
//! Rewrite matches the wire, not the report. Then, in legacy's `else if` order: the item must
//! exist (`GetItem` refuses a worn cell and anything at or past `CUSTOM_INVENTORY_SLOT_END`,
//! belt cells and dragon souls too), a combination takes costume and an absorption takes
//! costume in slot 0, a material held by another window chats `LOCKED_ITEMS`, slot 1 of a
//! combination takes the grade slot 0 holds (`SAME_GRADE`), a combination takes nothing at
//! the grade-4 absorption cap (`MAX_AB`), slot 1 of an absorption takes a weapon or a body
//! armour (`ABSORB_WEAPON_ARMOR`), and slot 0 of an absorption refuses nothing already
//! absorbed. The last two arms are silent (`COSTUME_SASH` in a combination and in slot 0 of
//! an absorption, the already-absorbed sash); `LC_TEXT` of an unknown key is the key itself,
//! so the chat keys go out raw.
//!
//! The landed add answers `ADDED` with the window flag, the added item's grade price, the
//! slot (1370's low byte under the custom inventory: `CheckEmptyMaterialSlot`'s
//! `CUSTOM_INVENTORY_SLOT_END` truncates to a `BYTE`, so the client hears 90 — pinned as a
//! Quirk), the named position, and the combine result. The result is
//! `GetSashCombineResult` (`:10039-10103`): a combination of two grade-4 sashes keeps the
//! vnum and widens the absorption by 5 up to 25; a lower combination steps the vnum up by
//! one when the proto table holds it (else it stays) and sets the grade's absorption — 5,
//! 10, or 11..=19; an absorption with both materials shows slot 0's vnum and absorption.
//! With fewer than two materials every number is 0.
//!
//! The remove (`RemoveSashMaterial`, `:10215-10255`) unlocks slot 1 alone, pricing what slot
//! 0 still holds, while slot 0 clears both; both answer `REMOVED` with a zeroed
//! `{INVENTORY, 0}` position and vnum 0. The refine (`RefineSashMaterials`, `:10313-10511`)
//! answers `REFINED` (the `SASH_SUBHEADER_CG_REFINED` value, 4, despite the `CG_` name): a
//! combination sends `pos` 0 and `max_absorb` 100 on success, 0 on failure; an absorption
//! sends `pos` 255 and `max_absorb` 1. A combination at no gold chats `NOT_ENOUGH_YANG` and
//! sends nothing. The gold leaves under `ChangeGold` (`ENABLE_REMOVE_LIMIT_GOLD`), so a
//! spend past the allowed gold refuses the whole step.
//!
//! A combination success creates the result (the masks are `CopyAllAttrTo`, so
//! `refined_item` builds it), rolls the absorption — `min` when the range is flat, else
//! `number(min + 1, max)` — carries slot 0's absorbed socket, spends the gold, destroys both
//! materials, and lands the new sash in slot 0's old cell; a grade-4 success chats the new
//! rate, any other the `SUCCESS` key, and the effect 25 goes around. A failure destroys
//! slot 1 alone and keeps its gold spent, and the unlock drops slot 1 while the record
//! keeps the window standing. An absorption copies slot 1's attributes onto slot 0
//! (`CopyAttributeTo`, then the negative ones forced to 0 and the refine element carried),
//! stamps slot 1's vnum into slot 0's absorbed socket, destroys slot 1, chats `SUCCESS`, and
//! clears the window. The money, item and sash logs are `sys.log`'s, which owns no sink.
//!
//! The detachment scroll (`USE_DETACHMENT` with `VALUE0` 7 on a sash, `CleanSashAttr`,
//! `:10481-10500`) rides the use-on-an-item path (`world::character::clean_sash_attr`), not
//! the window.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::item_slots::{
    EWindows, BELT_INVENTORY_SLOT_END, BELT_INVENTORY_SLOT_START, CUSTOM_INVENTORY_SLOT_END,
    INVENTORY_AND_EQUIP_SLOT_MAX,
};
use common::vid::Vid;
use gamedata::item_kind::{
    ARMOR_BODY, COSTUME_SASH, ITEM_ARMOR, ITEM_COSTUME, ITEM_DS, ITEM_WEAPON,
};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_sash::{
    GcSash, SASH_SUBHEADER_GC_ADDED, SASH_SUBHEADER_GC_CLOSE, SASH_SUBHEADER_GC_REFINED,
    SASH_SUBHEADER_GC_REMOVED,
};
use protocol::gc_vid::GcSpecialEffect;
use protocol::item_pos::ItemPos;
use world::character::{MoveDone, MoveKind, MoveRecord};
use world::item::ItemId;

use super::view::EntityKey;
use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::item_move::{gold_delta, MoveItemRefused, MovedItems, Mover};

/// `SASH_SUBHEADER_CG_CLOSE`: shut the window.
pub const SASH_CLOSE: u8 = 0;
/// `SASH_SUBHEADER_CG_ADD`: land a material.
pub const SASH_ADD: u8 = 1;
/// `SASH_SUBHEADER_CG_REMOVE`: lift a material.
pub const SASH_REMOVE: u8 = 2;
/// `SASH_SUBHEADER_CG_REFINE`: run the combine or the absorption.
pub const SASH_REFINE: u8 = 3;

/// `SASH_WINDOW_MAX_MATERIALS` (`C/item_length.h:519`): the window's two slots.
pub const SASH_MATERIALS: usize = 2;
/// `SASH_GRADE_VALUE_FIELD` (`C/item_length.h:503`): the grade lives in `VALUE0`.
pub const SASH_GRADE_VALUE: usize = 0;
/// `SASH_ABSORPTION_SOCKET` (`C/item_length.h:504`): socket 0.
pub const SASH_ABSORPTION_SOCKET: usize = 0;
/// `SASH_ABSORBED_SOCKET` (`C/item_length.h:505`): socket 1.
pub const SASH_ABSORBED_SOCKET: usize = 1;
/// `SASH_GRADE_2_ABS` (`C/item_length.h:507`): the grade-2 result's absorption.
pub const SASH_GRADE_2_ABS: u32 = 5;
/// `SASH_GRADE_3_ABS` (`C/item_length.h:508`): the grade-3 result's absorption.
pub const SASH_GRADE_3_ABS: u32 = 10;
/// `SASH_GRADE_4_ABS_MIN` (`C/item_length.h:509`): the grade-4 result's floor.
pub const SASH_GRADE_4_ABS_MIN: u32 = 11;
/// `SASH_GRADE_4_ABS_MAX` (`C/item_length.h:510`): the grade-4 absorption cap.
pub const SASH_GRADE_4_ABS_MAX: u32 = 25;
/// `SASH_GRADE_4_ABS_MAX_COMB` (`C/item_length.h:511`): the grade-3 combine's ceiling.
pub const SASH_GRADE_4_ABS_MAX_COMB: u32 = 19;
/// `SASH_GRADE_4_ABS_RANGE` (`C/item_length.h:512`): the grade-4 re-roll's width.
pub const SASH_GRADE_4_ABS_RANGE: u32 = 5;
/// `SASH_GRADE_1_PRICE` (`C/item_length.h:516`): the grade-1 (and default) price.
pub const SASH_GRADE_1_PRICE: u32 = 100_000;
/// `SASH_GRADE_2_PRICE` (`C/item_length.h:517`).
pub const SASH_GRADE_2_PRICE: u32 = 200_000;
/// `SASH_GRADE_3_PRICE` (`C/item_length.h:518`).
pub const SASH_GRADE_3_PRICE: u32 = 300_000;
/// `SASH_GRADE_4_PRICE` (`C/item_length.h:519`).
pub const SASH_GRADE_4_PRICE: u32 = 500_000;
/// `SASH_COMBINE_GRADE_1` (`C/item_length.h:520`): the grade-1 success percent.
pub const SASH_COMBINE_GRADE_1: i32 = 80;
/// `SASH_COMBINE_GRADE_2` (`C/item_length.h:521`).
pub const SASH_COMBINE_GRADE_2: i32 = 70;
/// `SASH_COMBINE_GRADE_3` (`C/item_length.h:522`).
pub const SASH_COMBINE_GRADE_3: i32 = 50;
/// `SASH_COMBINE_GRADE_4` (`C/item_length.h:523`).
pub const SASH_COMBINE_GRADE_4: i32 = 30;
/// `SE_EFFECT_SASH_SUCCEDED` (`C/length.h:879`): the combine success's effect.
pub const SE_EFFECT_SASH_SUCCEDED: u8 = 25;

/// The `ADDED` record's `bPos` under the custom inventory: legacy writes
/// `CUSTOM_INVENTORY_SLOT_END` (1370) into a `BYTE`, so the client hears 90.
pub const SASH_ADDED_POS_QUIRK: u8 = (CUSTOM_INVENTORY_SLOT_END & 0xff) as u8;
/// The empty-slot report `CheckEmptyMaterialSlot` cannot return: `bPos` 255 is the client's
/// "first free slot" spell, matching the wire legacy's `BYTE` reads.
pub const SASH_ANY_SLOT: u8 = 255;
/// `INVENTORY` in `EWindows`: the position the close, remove and refine records zero.
pub const SASH_ZERO_WINDOW: u8 = EWindows::Inventory as u8;

/// A material held by another window (`G/char.cpp:10142-10146`).
pub const SASH_LOCKED_NOTICE: &[u8] = b"LOCKED_ITEMS";
/// Slot 1 of a combination at another grade (`:10155-10159`).
pub const SASH_SAME_GRADE_NOTICE: &[u8] = b"SAME_GRADE";
/// A combination at the grade-4 absorption cap (`:10160-10164`).
pub const SASH_MAX_ABSORB_NOTICE: &[u8] = b"MAX_AB";
/// Slot 1 of an absorption that is no weapon or body armour (`:10165-10179`).
pub const SASH_ABSORB_KIND_NOTICE: &[u8] = b"ABSORB_WEAPON_ARMOR";
/// A combination at no gold (`:10341-10345`).
pub const SASH_POOR_NOTICE: &[u8] = b"NOT_ENOUGH_YANG";
/// A combine or absorption success (`:10387`, `:10496`).
pub const SASH_SUCCESS_NOTICE: &[u8] = b"SUCCESS";
/// A combine failure below grade 4 (`:10407`).
pub const SASH_FAILED_NOTICE: &[u8] = b"FAILED";
/// A grade-4 combine's new rate (`:10385`, `:10405`).
pub const SASH_NEW_RATE_NOTICE: &[u8] = b"NEW_ABSORPTION_RATE";

/// The sash window's two materials: the held item ids, slot by slot.
pub type SashMaterials = [Option<ItemId>; SASH_MATERIALS];

/// `GetSashCombinePrice` (`G/char.cpp:9992-10021`): the grade's price, grade 1's by default.
#[must_use]
pub const fn sash_combine_price(grade: i32) -> u32 {
    match grade {
        2 => SASH_GRADE_2_PRICE,
        3 => SASH_GRADE_3_PRICE,
        4 => SASH_GRADE_4_PRICE,
        _ => SASH_GRADE_1_PRICE,
    }
}

/// The success percent of a combination at `grade` (`G/char.cpp:10324-10340`).
#[must_use]
pub const fn sash_combine_chance(grade: i32) -> i32 {
    match grade {
        2 => SASH_COMBINE_GRADE_2,
        3 => SASH_COMBINE_GRADE_3,
        4 => SASH_COMBINE_GRADE_4,
        _ => SASH_COMBINE_GRADE_1,
    }
}

impl GameState {
    /// Whether the sash window of the character under `vid` stands in combination mode.
    /// `None` when no window stands.
    pub(super) fn sash_combination(&self, vid: Vid) -> Option<bool> {
        self.sash_windows.get(&vid).copied()
    }

    /// The materials the sash window of the character under `vid` holds, slot by slot.
    /// Empty slots are `None`; a closed window holds nothing.
    pub(super) fn sash_materials(&self, vid: Vid) -> SashMaterials {
        self.sash_held.get(&vid).copied().unwrap_or([None, None])
    }

    /// Whether the item `id` is a material of any open sash window: `CItem::isLocked` as
    /// the sash sets it. The toggle and soul locks are not ported, so the sash is the only
    /// writer.
    pub(super) fn sash_locked(&self, id: ItemId) -> bool {
        self.sash_held.values().any(|held| held.contains(&Some(id)))
    }

    /// Whether the item at `at` of the character under `vid` is a material of its open
    /// sash window: the `isLocked` check the destroy, drop, move, gift, shop, safebox and
    /// scroll-refine paths run.
    pub(super) fn sash_item_at(&self, vid: Vid, at: ItemPos) -> bool {
        let held = self.sash_materials(vid);
        if held == [None, None] {
            return false;
        }
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .is_some_and(|item| held.contains(&Some(item.id)))
    }

    /// Run one `CG_SASH` for the character online under `vid` (`CInputMain::Sash`,
    /// `G/input_main.cpp:3395-3425`): close, add, remove, or refine. A waiting quest and
    /// an unknown sub-header are absorbed, as legacy's guard and `default` do.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn sash(
        &mut self,
        vid: Vid,
        sub_header: u8,
        (pos, at): (u8, ItemPos),
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
        if self.quest_running(vid) {
            return Ok(moved);
        }
        match sub_header {
            SASH_CLOSE => self.close_sash(vid, &mut moved),
            SASH_ADD => self.add_sash_material(vid, pos, at, actor, &mut moved),
            SASH_REMOVE => self.remove_sash_material(vid, pos, &mut moved),
            SASH_REFINE => self.refine_sash_materials(vid, actor, &mut moved),
            _ => {}
        }
        Ok(moved)
    }

    /// `CloseSash` (`G/char.cpp:9936-9965`): without a window nothing is sent; else both
    /// materials unlock and the close record answers.
    fn close_sash(&mut self, vid: Vid, moved: &mut MovedItems) {
        let Some(combination) = self.sash_windows.remove(&vid) else {
            return;
        };
        self.sash_held.remove(&vid);
        let close = GcSash::new(
            SASH_SUBHEADER_GC_CLOSE,
            combination,
            0,
            0,
            protocol::cg_sash::ItemPos::new(SASH_ZERO_WINDOW, 0),
            0,
            0,
            0,
        );
        moved.records.push(close.encode());
    }

    /// `AddSashMaterial` (`G/char.cpp:10105-10213`): the slot resolution and the guard
    /// chain in legacy's order, then the held id and the `ADDED` record.
    fn add_sash_material(
        &mut self,
        vid: Vid,
        pos: u8,
        at: ItemPos,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        let Some(combination) = self.sash_combination(vid) else {
            return;
        };
        let mut held = self.sash_materials(vid);
        let Some(slot) = Self::sash_slot(&held, pos) else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let Some(item) = character.items().item_at(at).cloned() else {
            return;
        };
        if u32::from(at.cell) >= u32::from(INVENTORY_AND_EQUIP_SLOT_MAX)
            || at.window_type == EWindows::Equipment as u8
            || (at.cell >= BELT_INVENTORY_SLOT_START && at.cell < BELT_INVENTORY_SLOT_END)
        {
            return;
        }
        let Some(proto) = self.protos.get(item.vnum).cloned() else {
            return;
        };
        if proto.item_type == ITEM_DS {
            return;
        }
        let grade = proto.values[SASH_GRADE_VALUE];
        let absorb = item.sockets[SASH_ABSORPTION_SOCKET];
        let absorbed = item.sockets[SASH_ABSORBED_SOCKET];
        if proto.item_type != ITEM_COSTUME && (combination || slot == 0) {
            return;
        }
        if self.sash_locked(item.id) || self.shaman_locked(vid, item.id) {
            moved
                .records
                .push(self.sash_line(actor, SASH_LOCKED_NOTICE, &[]));
            return;
        }
        let grade0 = held[0].and_then(|id| self.sash_grade(vid, id));
        if combination && slot == 1 && grade0 != Some(grade) {
            moved
                .records
                .push(self.sash_line(actor, SASH_SAME_GRADE_NOTICE, &[]));
            return;
        }
        if combination && u32::try_from(absorb).unwrap_or(0) >= SASH_GRADE_4_ABS_MAX {
            moved
                .records
                .push(self.sash_line(actor, SASH_MAX_ABSORB_NOTICE, &[]));
            return;
        }
        if slot == 1 && !combination && !Self::sash_absorbs(&proto) {
            moved
                .records
                .push(self.sash_line(actor, SASH_ABSORB_KIND_NOTICE, &[]));
            return;
        }
        if proto.sub_type != COSTUME_SASH && (combination || slot == 0) {
            return;
        }
        if absorbed > 0 && slot == 0 && !combination {
            return;
        }
        if slot == 1 && held[0].is_none() {
            return;
        }
        if held[slot].is_some() {
            return;
        }
        held[slot] = Some(item.id);
        self.sash_held.insert(vid, held);
        let (vnum, min, max) = self.sash_combine_result(vid, combination);
        let added = GcSash::new(
            SASH_SUBHEADER_GC_ADDED,
            combination,
            sash_combine_price(grade),
            SASH_ADDED_POS_QUIRK,
            protocol::cg_sash::ItemPos::new(at.window_type, at.cell),
            vnum,
            min,
            max,
        );
        moved.records.push(added.encode());
    }

    /// The request's slot: a slot below 2, or the first free slot when the client sent
    /// 255 (`AddSashMaterial`, `G/char.cpp:10105-10119`). `None` for any other `pos`.
    fn sash_slot(held: &SashMaterials, pos: u8) -> Option<usize> {
        if usize::from(pos) < SASH_MATERIALS {
            return Some(usize::from(pos));
        }
        if pos != SASH_ANY_SLOT {
            return None;
        }
        held.iter().position(Option::is_none)
    }

    /// Whether slot 1 of an absorption takes the proto: a weapon or a body armour
    /// (`AddSashMaterial`, `G/char.cpp:10165-10179`).
    fn sash_absorbs(proto: &gamedata::item_proto::ItemProto) -> bool {
        proto.item_type == ITEM_WEAPON
            || (proto.item_type == ITEM_ARMOR && proto.sub_type == ARMOR_BODY)
    }

    /// The grade of the material `id` of the character under `vid`: its proto `VALUE0`.
    fn sash_grade(&self, vid: Vid, id: ItemId) -> Option<i32> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let item = character.items().item(id)?;
        self.protos
            .get(item.vnum)
            .map(|proto| proto.values[SASH_GRADE_VALUE])
    }

    /// A sash `LC_TEXT` key chat as an `INFO` line. The keys name no locale row, so the
    /// raw key goes out.
    fn sash_line(&self, actor: Mover, key: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
        chat_packet(actor.recipient(&self.locale), CHAT_TYPE_INFO, key, args)
    }
}

impl GameState {
    /// `GetSashCombineResult` (`G/char.cpp:10039-10103`): the result vnum and the absorption
    /// range for the window's materials. Anything short of two materials zeroes every number.
    fn sash_combine_result(&self, vid: Vid, combination: bool) -> (u32, u32, u32) {
        let held = self.sash_materials(vid);
        let (Some(first), Some(_)) = (held[0], held[1]) else {
            return (0, 0, 0);
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return (0, 0, 0);
        };
        let Some(item) = character.items().item(first) else {
            return (0, 0, 0);
        };
        let absorb = u32::try_from(item.sockets[SASH_ABSORPTION_SOCKET]).unwrap_or(0);
        if !combination {
            return (item.vnum, absorb, absorb);
        }
        let grade = self
            .protos
            .get(item.vnum)
            .map_or(0, |proto| proto.values[SASH_GRADE_VALUE]);
        if grade == 4 {
            return (
                item.vnum,
                absorb,
                (absorb + SASH_GRADE_4_ABS_RANGE).min(SASH_GRADE_4_ABS_MAX),
            );
        }
        let stepped = item.vnum.saturating_add(1);
        let vnum = if self.protos.get(stepped).is_some() {
            stepped
        } else {
            item.vnum
        };
        match grade {
            2 => (vnum, SASH_GRADE_3_ABS, SASH_GRADE_3_ABS),
            3 => (vnum, SASH_GRADE_4_ABS_MIN, SASH_GRADE_4_ABS_MAX_COMB),
            _ => (vnum, SASH_GRADE_2_ABS, SASH_GRADE_2_ABS),
        }
    }

    /// `RemoveSashMaterial` (`G/char.cpp:10215-10255`): slot 1 unlocks alone, pricing what
    /// slot 0 still holds; slot 0 clears both. Both answer `REMOVED` with a zeroed position.
    fn remove_sash_material(&mut self, vid: Vid, pos: u8, moved: &mut MovedItems) {
        let Some(combination) = self.sash_combination(vid) else {
            return;
        };
        if usize::from(pos) >= SASH_MATERIALS {
            return;
        }
        let mut held = self.sash_materials(vid);
        let mut price = 0;
        if pos == 1 {
            held[1] = None;
            if let Some(first) = held[0] {
                if let Some(grade) = self.sash_grade(vid, first) {
                    price = sash_combine_price(grade);
                }
            }
        } else {
            held = [None, None];
        }
        self.sash_held.insert(vid, held);
        let removed = GcSash::new(
            SASH_SUBHEADER_GC_REMOVED,
            combination,
            price,
            pos,
            protocol::cg_sash::ItemPos::new(SASH_ZERO_WINDOW, 0),
            0,
            0,
            0,
        );
        moved.records.push(removed.encode());
    }

    /// `RefineSashMaterials` (`G/char.cpp:10313-10511`): the guard, then the combine or
    /// the absorption. Without a window there is nothing to refine.
    fn refine_sash_materials(&mut self, vid: Vid, actor: Mover, moved: &mut MovedItems) {
        let Some(combination) = self.sash_combination(vid) else {
            return;
        };
        if self.sash_can_refine(vid, combination) == 0 {
            return;
        }
        if combination {
            self.refine_sash_combine(vid, actor, moved);
        } else {
            self.refine_sash_absorb(vid, actor, moved);
        }
    }

    /// `CanRefineSashMaterials` (`G/char.cpp:10257-10311`): 1 for a combination of two
    /// sashes, 2 for an absorption of a sash and a weapon or body armour onto a sash with
    /// no absorbed socket, else 0.
    fn sash_can_refine(&self, vid: Vid, combination: bool) -> u8 {
        let held = self.sash_materials(vid);
        let (Some(first), Some(second)) = (held[0], held[1]) else {
            return 0;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return 0;
        };
        let items = character.items();
        let (Some(one), Some(two)) = (items.item(first), items.item(second)) else {
            return 0;
        };
        let (Some(proto0), Some(proto1)) = (self.protos.get(one.vnum), self.protos.get(two.vnum))
        else {
            return 0;
        };
        let sash0 = proto0.item_type == ITEM_COSTUME && proto0.sub_type == COSTUME_SASH;
        if combination {
            let sash1 = proto1.item_type == ITEM_COSTUME && proto1.sub_type == COSTUME_SASH;
            return u8::from(sash0 && sash1);
        }
        if !sash0 || !Self::sash_absorbs(proto1) {
            return 0;
        }
        if one.sockets[SASH_ABSORBED_SOCKET] > 0 {
            return 0;
        }
        2
    }

    /// The combination arm of `RefineSashMaterials` (`G/char.cpp:10318-10494`): the poor
    /// chat sends no record; a success builds the result, spends the gold, destroys both
    /// materials and lands the new sash in slot 0's old cell with the effect around; a
    /// failure spends the gold, destroys slot 1 alone, and keeps the window standing.
    fn refine_sash_combine(&mut self, vid: Vid, actor: Mover, moved: &mut MovedItems) {
        let [Some(first), Some(second)] = self.sash_materials(vid) else {
            return;
        };
        let grade = self.sash_grade(vid, first).unwrap_or(0);
        let price = sash_combine_price(grade);
        let (vnum, min, max) = self.sash_combine_result(vid, true);
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        if character.gold() < u64::from(price) {
            moved
                .records
                .push(self.sash_line(actor, SASH_POOR_NOTICE, &[]));
            return;
        }
        let chance = sash_combine_chance(grade);
        let draw = world::character::number(&mut self.dice, 1, 100);
        if draw <= chance {
            self.sash_combine_success(vid, (first, second), (vnum, min, max), price, actor, moved);
        } else {
            self.sash_combine_failure(vid, second, price, actor, moved);
        }
    }

    /// The combination success: the result built by `CopyAllAttrTo`, the rolled absorption,
    /// the spent gold, both materials destroyed, the new sash in slot 0's old cell, the rate
    /// or success chat, the effect around, and the cleared window — then the `REFINED`
    /// record with `pos` 0 and `max_absorb` 100.
    fn sash_combine_success(
        &mut self,
        vid: Vid,
        (first, second): (ItemId, ItemId),
        (vnum, min, max): (u32, u32, u32),
        price: u32,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let Some(old) = character.items().item(first).cloned() else {
            return;
        };
        let cell = old.pos.cell;
        let window = old.pos.window_type;
        let Some(old_proto) = self.protos.get(old.vnum).cloned() else {
            return;
        };
        let Some(made) = self.protos.get(vnum).cloned() else {
            tracing::error!("cannot create item {vnum}");
            return;
        };
        let Some(id) = self.sash_result_id(vnum) else {
            return;
        };
        let (made, at, absorb, grade) = self.sash_made_item(
            &old,
            &old_proto,
            (id, vnum),
            &made,
            (min, max),
            (window, cell),
        );
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let held = character.gold();
        let left = held - u64::from(price);
        character.set_gold(left);
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let pid = moved.owner_id;
        let Some(pos1) = items.cell_of(second) else {
            return;
        };
        let mut gone = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        gone.records
            .push(MoveRecord::QuickslotSync(world::character::QuickslotSync {
                from: pos1.cell,
                to: world::character::SyncTo::Delete,
            }));
        if items.release(second).is_err() {
            return;
        }
        gone.records
            .push(MoveRecord::Item(world::character::ItemRecord::Set(
                world::item::gc_item_clear(pos1),
            )));
        gone.changes
            .push(world::character::ItemChange::Destroyed { id: second });
        let mut swapped = world::character::swap_refined(items, at, Some(&made));
        swapped.records.push(MoveRecord::Gold {
            amount: 0,
            value: left,
        });
        world::character::sync_quickslots(&mut gone, slots, items);
        world::character::sync_quickslots(&mut swapped, slots, items);
        let quickslots = character.quickslots().clone();
        let points = character.points().cloned();
        let gone = MovedItems::new(pid, vid.raw(), gone, actor, &self.protos, &self.locale);
        let swapped = MovedItems::new(pid, vid.raw(), swapped, actor, &self.protos, &self.locale);
        moved.records.extend(gone.records);
        moved.records.extend(swapped.records);
        moved.changes.extend(gone.changes);
        moved.changes.extend(swapped.changes);
        moved.gold = Some(gold_delta(held, left));
        moved.points = points;
        moved.quickslots = Some(quickslots);
        if grade == 4 {
            moved.records.push(self.sash_line(
                actor,
                SASH_NEW_RATE_NOTICE,
                &[Arg::Int(i64::from(absorb))],
            ));
        } else {
            moved
                .records
                .push(self.sash_line(actor, SASH_SUCCESS_NOTICE, &[]));
        }
        self.sash_success_close(vid);
        let refined = GcSash::new(
            SASH_SUBHEADER_GC_REFINED,
            true,
            price,
            0,
            protocol::cg_sash::ItemPos::new(SASH_ZERO_WINDOW, 0),
            0,
            0,
            100,
        );
        moved.records.push(refined.encode());
    }

    /// A fresh item id for the combination result, or `None` with the `cannot create`
    /// error logged when the pool is gone.
    fn sash_result_id(&mut self, vnum: u32) -> Option<ItemId> {
        let ids = self.item_ids.as_mut()?;
        let Ok(id) = ids.allocate() else {
            tracing::error!("cannot create item {vnum}");
            return None;
        };
        Some(id)
    }

    /// Build the combination result: `CopyAllAttrTo` through `refined_item`, the rolled
    /// absorption (`min` when the range is flat, else `number(min + 1, max)`), slot 0's
    /// absorbed socket carried, in slot 0's old cell. Answers the item, its cell, the
    /// absorption and the grade.
    fn sash_made_item(
        &mut self,
        old: &world::item::Item,
        old_proto: &gamedata::item_proto::ItemProto,
        (id, vnum): (ItemId, u32),
        made: &gamedata::item_proto::ItemProto,
        (min, max): (u32, u32),
        (window, cell): (u8, u16),
    ) -> (world::item::Item, ItemPos, u32, i32) {
        let mut made = world::character::refined_item(
            old,
            old_proto,
            (id, vnum),
            made,
            i64::from(Self::global_time()),
        );
        let absorb = if min == max {
            min
        } else {
            let from = i32::try_from(min.saturating_add(1)).unwrap_or(i32::MAX);
            let to = i32::try_from(max).unwrap_or(i32::MAX);
            u32::try_from(world::character::number(&mut self.dice, from, to)).unwrap_or(min)
        };
        made.sockets[SASH_ABSORPTION_SOCKET] = i32::try_from(absorb).unwrap_or(i32::MAX);
        made.sockets[SASH_ABSORBED_SOCKET] = old.sockets[SASH_ABSORBED_SOCKET];
        let at = ItemPos::new(window, cell);
        made.pos = at;
        (made, at, absorb, old_proto.values[SASH_GRADE_VALUE])
    }

    /// The success's close: the effect 25 around (which reaches the mover too), then the
    /// window forgotten.
    fn sash_success_close(&mut self, vid: Vid) {
        let mut effect = Vec::new();
        GcSpecialEffect {
            effect_type: SE_EFFECT_SASH_SUCCEDED,
            vid: vid.raw(),
        }
        .encode_into(&mut effect);
        self.packet_around(EntityKey::Character(vid.raw()), &effect, None);
        self.sash_windows.remove(&vid);
        self.sash_held.remove(&vid);
    }

    /// The combination failure: the gold spent, slot 1 destroyed and unlocked, the rate or
    /// failure chat, and the `REFINED` record with `pos` 0 and `max_absorb` 0.
    fn sash_combine_failure(
        &mut self,
        vid: Vid,
        second: ItemId,
        price: u32,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        let rate = self.sash_materials(vid)[0]
            .and_then(|first| {
                self.characters
                    .find_by_vid(vid)
                    .ok()
                    .and_then(|character| character.items().item(first).cloned())
            })
            .map_or(0, |item| item.sockets[SASH_ABSORPTION_SOCKET]);
        let grade = self.sash_materials(vid)[0]
            .and_then(|first| self.sash_grade(vid, first))
            .unwrap_or(0);
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let held = character.gold();
        let left = held - u64::from(price);
        character.set_gold(left);
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let Some(pos) = items.cell_of(second) else {
            return;
        };
        let mut gone = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        gone.records
            .push(MoveRecord::QuickslotSync(world::character::QuickslotSync {
                from: pos.cell,
                to: world::character::SyncTo::Delete,
            }));
        if items.release(second).is_err() {
            return;
        }
        gone.records
            .push(MoveRecord::Item(world::character::ItemRecord::Set(
                world::item::gc_item_clear(pos),
            )));
        gone.changes
            .push(world::character::ItemChange::Destroyed { id: second });
        gone.records.push(MoveRecord::Gold {
            amount: 0,
            value: left,
        });
        world::character::sync_quickslots(&mut gone, slots, items);
        let quickslots = character.quickslots().clone();
        let points = character.points().cloned();
        let pid = moved.owner_id;
        let gone = MovedItems::new(pid, vid.raw(), gone, actor, &self.protos, &self.locale);
        moved.records.extend(gone.records);
        moved.changes.extend(gone.changes);
        moved.gold = Some(gold_delta(held, left));
        moved.points = points;
        moved.quickslots = Some(quickslots);
        if grade == 4 {
            moved.records.push(self.sash_line(
                actor,
                SASH_NEW_RATE_NOTICE,
                &[Arg::Int(i64::from(rate))],
            ));
        } else {
            moved
                .records
                .push(self.sash_line(actor, SASH_FAILED_NOTICE, &[]));
        }
        let mut held = self.sash_materials(vid);
        held[1] = None;
        self.sash_held.insert(vid, held);
        let refined = GcSash::new(
            SASH_SUBHEADER_GC_REFINED,
            true,
            price,
            0,
            protocol::cg_sash::ItemPos::new(SASH_ZERO_WINDOW, 0),
            0,
            0,
            0,
        );
        moved.records.push(refined.encode());
    }

    /// The absorption arm of `RefineSashMaterials` (`G/char.cpp:10496-10511`): slot 1's
    /// attributes onto slot 0 (`CopyAttributeTo`, then the negative forced to 0 and the
    /// refine element carried), slot 1's vnum into slot 0's absorbed socket, slot 1
    /// destroyed, the `SUCCESS` chat, the cleared window, and the `REFINED` record with
    /// `pos` 255 and `max_absorb` 1.
    fn refine_sash_absorb(&mut self, vid: Vid, actor: Mover, moved: &mut MovedItems) {
        let [Some(first), Some(second)] = self.sash_materials(vid) else {
            return;
        };
        let price = self
            .sash_grade(vid, first)
            .map_or(SASH_GRADE_1_PRICE, sash_combine_price);
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let (Some(one), Some(two)) = (
            character.items().item(first).cloned(),
            character.items().item(second).cloned(),
        ) else {
            return;
        };
        let mut one = one;
        one.attributes = two.attributes;
        for attribute in &mut one.attributes {
            if attribute.s_value < 0 {
                attribute.s_value = 0;
            }
        }
        one.sockets[SASH_ABSORBED_SOCKET] = i32::try_from(two.vnum).unwrap_or(i32::MAX);
        if two.refine_element != 0 {
            one.refine_element = two.refine_element;
        }
        let (attributes, sockets) = (one.attributes, one.sockets);
        let update = one.gc_item_update();
        let pos1 = two.pos;
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        for (index, attribute) in attributes.iter().enumerate() {
            let _ = items.set_attribute(first, index, *attribute);
        }
        let absorbed = i32::try_from(two.vnum).unwrap_or(i32::MAX);
        let _ = items.set_socket(first, SASH_ABSORBED_SOCKET, absorbed);

        if two.refine_element != 0 {
            let _ = items.set_refine_element(first, two.refine_element);
        }
        let mut done = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        done.records
            .push(MoveRecord::Item(world::character::ItemRecord::Update(
                update,
            )));
        done.changes.push(world::character::ItemChange::Attributes {
            id: first,
            attributes,
        });
        done.changes
            .push(world::character::ItemChange::Sockets { id: first, sockets });
        done.records
            .push(MoveRecord::QuickslotSync(world::character::QuickslotSync {
                from: pos1.cell,
                to: world::character::SyncTo::Delete,
            }));
        if items.release(second).is_ok() {
            done.records
                .push(MoveRecord::Item(world::character::ItemRecord::Set(
                    world::item::gc_item_clear(pos1),
                )));
            done.changes
                .push(world::character::ItemChange::Destroyed { id: second });
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
        moved
            .records
            .push(self.sash_line(actor, SASH_SUCCESS_NOTICE, &[]));
        self.sash_windows.remove(&vid);
        self.sash_held.remove(&vid);
        let refined = GcSash::new(
            SASH_SUBHEADER_GC_REFINED,
            false,
            price,
            SASH_ANY_SLOT,
            protocol::cg_sash::ItemPos::new(SASH_ZERO_WINDOW, 0),
            0,
            0,
            1,
        );
        moved.records.push(refined.encode());
    }
}
