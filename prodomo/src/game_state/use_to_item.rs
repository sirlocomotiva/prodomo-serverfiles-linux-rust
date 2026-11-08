//! An item used on another: `CG_ITEM_USE_TO_ITEM` (`CInputMain::ItemToItem`,
//! `G/input_main.cpp:1006-1022`) runs `CHARACTER::UseItem(Cell, DestCell)`
//! (`G/char_item.cpp:7168-7400`), whose `UseItemEx` arms read the second cell
//! (`:2718-6990`).
//!
//! Two arms read it here: a stone (`ITEM_METIN`) goes into the target's socket
//! (`world::character::put_stone`), and a refine scroll (`USE_TUNING`) opens the refine
//! window for the target (`CHARACTER::RefineItem`, `:1970-2041`), which `CG_REFINE` answers
//! with `DoRefineWithScroll` (`:1595-1862`). Every other item is used as `CG_ITEM_USE` uses
//! it: legacy's other arms ignore the second cell, and the arms that read it are not ported
//! (`USE_DETACHMENT`, the bonus and socket scrolls, the costume and talisman scrolls, the
//! hair).
//!
//! A sash cleaning scroll (`USE_DETACHMENT` with `VALUE0` 7) cleans the sash's absorbed
//! socket and attributes (`CleanSashAttr`, `G/char.cpp:10481-10500`); a changelook's
//! (`VALUE0` 8) cleans the transmutation (`CleanTransmutation`, `G/char.cpp:10746-10768`).
//! The owner's protos have neither scroll. An element scroll (`USE_ELEMENT_UPGRADE`,
//! `USE_ELEMENT_DOWNGRADE`, `USE_ELEMENT_CHANGE`) stands the element window on the
//! target (`RefineElementInformation`, `G/char_item.cpp:10452-10472`), which
//! `CG_REFINE_ELEMENT` answers.
//!
//!
//! Divergence: a scroll or a target outside the inventory window opens no window, where
//! legacy passes the cell number to `RefineInformation`, which reads the inventory cell of
//! that number (the same Divergence as the smith's gift; `docs/STATUS.md`).
//!
//! `G/` = `server/server/game/`.

use std::ops::RangeInclusive;

use common::item_slots::EWindows;
use common::vid::Vid;
use gamedata::item_kind::{
    ITEM_METIN, ITEM_USE, USE_DETACHMENT, USE_ELEMENT_CHANGE, USE_ELEMENT_DOWNGRADE,
    USE_ELEMENT_UPGRADE, USE_TUNING,
};
use gamedata::item_proto::ItemProto;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::item_pos::ItemPos;
use world::character::{MoveDone, MoveKind, MoveRefused, Unported};

use super::smith_refine::{
    FailOutcome, RefinePlan, NOT_REFINABLE_NOTICE, NO_RESULT_NOTICE, REFINE_GOLD_NOTICE,
    REFINE_MATERIAL_NOTICE, REFINE_TYPE_BDRAGON, REFINE_TYPE_HYUNIRON, REFINE_TYPE_MUSIN,
    REFINE_TYPE_SCROLL,
};
use super::GameState;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// `HYUNIRON_CHN`: magic iron, which keeps the recipe's chance.
const HYUNIRON_CHN: i32 = 1;
/// `YONGSIN_SCROLL`: the dragon god's blessing, whose chance falls with the refine level.
const YONGSIN_SCROLL: i32 = 2;
/// `MUSIN_SCROLL`: the war god's blessing, sure below +4.
const MUSIN_SCROLL: i32 = 3;
/// `YAGONG_SCROLL`: the blacksmith's handbook, whose chance falls with the refine level.
const YAGONG_SCROLL: i32 = 4;
/// `MEMO_SCROLL`: a scroll sure at the one refine level its `VALUE1` names.
const MEMO_SCROLL: i32 = 5;
/// `BDRAGON_SCROLL`: the black dragon scroll, for +4 stones.
const BDRAGON_SCROLL: i32 = 6;
/// `SASH_CLEAN_ATTR_VALUE0` (`C/item_length.h:514`).
const SASH_CLEAN_ATTR_VALUE0: i32 = 7;
/// `CL_CLEAN_ATTR_VALUE0` (`C/length.h:179`).
const CL_CLEAN_ATTR_VALUE0: i32 = 8;

/// The +3 stones, which no scroll refines (`G/char_item.cpp:5993-5997`).
const PLUS_THREE_STONES: RangeInclusive<u32> = 28_330..=28_343;
/// The +4 stones, which only [`STONE_SCROLL`] refines (`:5999-6009`).
const PLUS_FOUR_STONES: RangeInclusive<u32> = 28_430..=28_443;
/// The scroll that refines a +4 stone.
const STONE_SCROLL: u32 = 71_056;
/// A +3 stone under a scroll.
const PLUS_THREE_NOTICE: &[u8] = b"[#Unk] Stones +3 cannot be improved with this item.";
/// A +4 stone under another scroll; legacy's text has the double space.
const PLUS_FOUR_NOTICE: &[u8] = b"Stones cannot be improved with  this item.";
/// The refine set a black dragon scroll refines (`:1987`).
const BDRAGON_SET: u16 = 702;
/// The refine set a plain scroll does not refine (`:1992`).
const NO_SCROLL_SET: u16 = 501;
/// A war god's blessing or a black dragon scroll on an item of the wrong level.
const SCROLL_LEVEL_NOTICE: &[u8] = b"[LS;1056]";
/// A memo scroll on an item of another level.
const MEMO_LEVEL_NOTICE: &[u8] = b"[LS;162]";
/// The dragon god's blessing's chance by refine level (`hyuniron_prob`, `:1729`).
const YONGSIN_PROB: [i32; 9] = [100, 75, 65, 55, 45, 40, 35, 25, 20];
/// The handbook's chance by refine level (`yagong_prob`, `:1730`).
const YAGONG_PROB: [i32; 9] = [100, 100, 90, 80, 70, 60, 50, 30, 20];

/// `CItem::GetRefineLevel` (`G/item.cpp:1869-1894`): the number after the last `+` of the
/// proto's base name, read as `str_to_number` (`strtol`) reads it; 0 without a `+`.
#[must_use]
pub fn refine_level(proto: &ItemProto) -> i32 {
    let Some(plus) = proto.name.iter().rposition(|&byte| byte == b'+') else {
        return 0;
    };
    let rest = &proto.name[plus + 1..];
    let rest = rest
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map_or(&[][..], |start| &rest[start..]);
    let (negative, digits) = match rest.first() {
        Some(b'-') => (true, &rest[1..]),
        Some(b'+') => (false, &rest[1..]),
        _ => (false, rest),
    };
    let value = digits
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .fold(0_i64, |value, digit| {
            (value * 10 + i64::from(digit - b'0')).min(i64::from(i32::MAX) + 1)
        });
    let value = if negative { -value } else { value };
    i32::try_from(value.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0)
}

/// `IsLimitedPotion`'s complete list: the range subsumes its duplicated switch cases.
fn arena_limited_item(vnum: u32) -> bool {
    (50_801..=50_826).contains(&vnum)
        || (50_020..=50_022).contains(&vnum)
        || matches!(vnum, 71_044 | 71_055)
}

/// Arena potion admission (`G/char_item.cpp:5581-5609`, `:5684-5714`).
fn arena_potion_refusal(
    vnum: u32,
    no_delay: bool,
    disabled: i32,
    limit: i32,
    left: i32,
) -> Option<&'static [u8]> {
    let allowed = if no_delay {
        matches!(vnum, 70_020 | 71_018 | 71_019 | 71_020)
    } else {
        (27_001..=27_006).contains(&vnum)
    };
    if disabled > 0 || !allowed {
        Some(b"[LS;403]")
    } else if limit < 10_000 && left <= 0 {
        Some(b"[LS;122]")
    } else {
        None
    }
}

/// The arm of `UseItemEx` an item used on another runs.
enum Arm {
    /// `ITEM_METIN`.
    Stone,
    /// `USE_TUNING` with its `VALUE0`.
    Scroll(i32),
    /// `USE_DETACHMENT` with `VALUE0` 7: a sash cleaning scroll.
    CleanSash,
    /// `USE_DETACHMENT` with `VALUE0` 8: a transmutation cleaning scroll.
    CleanLook,
    /// `USE_ELEMENT_UPGRADE`, `USE_ELEMENT_DOWNGRADE` or `USE_ELEMENT_CHANGE`: an
    /// element scroll, with its sub-type.
    Element(i32),
    /// Any other item, used as `CG_ITEM_USE` uses it.
    Plain,
}

impl GameState {
    pub(super) fn arena_item_refused(
        &mut self,
        vid: Vid,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        self.run_item_step(vid, actor, |_, _, _, _, _| {
            Ok(MoveDone {
                kind: MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            })
        })
    }

    pub(super) fn arena_potion_at(&self, vid: Vid, at: ItemPos) -> Option<(u32, i32)> {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .and_then(|item| self.protos.get(item.vnum))
            .filter(|proto| proto.item_type == gamedata::item_kind::ITEM_USE)
            .map(|proto| (proto.vnum, proto.sub_type))
    }

    /// The arena gates shared by plain use, use-on-item and quest-use dispatch
    /// (`G/char_item.cpp:2764-2768`, `G/arena.cpp:1131-1170`). These read the map,
    /// not arena membership: a visitor is restricted too.
    pub(super) fn arena_item_use_allowed(&mut self, vid: Vid, at: ItemPos) -> bool {
        let Some((vnum, item_type, sub_type)) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .and_then(|item| {
                self.protos
                    .get(item.vnum)
                    .map(|proto| (item.vnum, proto.item_type, proto.sub_type))
            })
        else {
            return true;
        };
        let arena_map = self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| self.is_arena_map(map));
        if arena_map && arena_limited_item(vnum) {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1205]", &[]);
            }
            return false;
        }
        if item_type == ITEM_USE
            && matches!(
                sub_type,
                gamedata::item_kind::USE_POTION | gamedata::item_kind::USE_POTION_NODELAY
            )
        {
            return self.arena_use_potion_allowed(
                vid,
                vnum,
                sub_type == gamedata::item_kind::USE_POTION_NODELAY,
            );
        }
        true
    }

    /// Map-based potion limits are independent of duelist binding.
    pub(super) fn arena_use_potion_allowed(&mut self, vid: Vid, vnum: u32, no_delay: bool) -> bool {
        if !self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| self.is_arena_map(map))
        {
            return true;
        }
        let refused = arena_potion_refusal(
            vnum,
            no_delay,
            self.arena_event_flag(b"arena_potion_limit"),
            self.arena_event_flag(b"arena_potion_limit_count"),
            self.arena_potion_count(vid),
        );
        if let Some(line) = refused {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_INFO, line, &[]);
            }
            return false;
        }
        true
    }

    /// Both accepted potion arms decrement even outside arena maps. A declined standard
    /// potion may have added SP recovery before refusing HP, but keeps its item and count
    /// (`G/char_item.cpp:3523-3540`, `:5660-5677`, `:5723-5754`).
    pub(super) fn arena_consumed_potion(
        &mut self,
        vid: Vid,
        _vnum: u32,
        sub_type: i32,
        moved: &MovedItems,
    ) {
        if moved.kind == MoveKind::Used
            && matches!(
                sub_type,
                gamedata::item_kind::USE_POTION | gamedata::item_kind::USE_POTION_NODELAY
            )
            && moved.changes.iter().any(|change| {
                matches!(
                    change,
                    db::items::RowChange::Destroyed { .. } | db::items::RowChange::Count { .. }
                )
            })
        {
            self.consume_arena_potion(vid);
        }
    }

    /// Run one `CG_ITEM_USE_TO_ITEM` for the character online under `vid`: the item at `at`
    /// used on the item at `dest`.
    ///
    /// # Errors
    ///
    /// As [`Self::use_item`].
    pub fn use_item_to_item(
        &mut self,
        vid: Vid,
        at: ItemPos,
        dest: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let arm = self.use_arm(vid, at);
        if matches!(arm, Arm::Plain) {
            return self.use_item(
                vid,
                at,
                actor,
                &crate::game_loop_messages::QuestAccount::default(),
            );
        }
        if self.is_down(vid.raw()) {
            return Err(MoveItemRefused::Refused(MoveRefused::UsedWhileDown));
        }
        if !self.arena_item_use_allowed(vid, at) {
            return self.run_item_step(vid, actor, |_, _, _, _, _| {
                Ok(MoveDone {
                    kind: MoveKind::Used,
                    records: Vec::new(),
                    changes: Vec::new(),
                })
            });
        }
        // `CanHandleItem()`: the windows this build has, a smith's among them.
        if self.sash_windows.contains_key(&vid)
            || self.changelook_windows.contains(&vid)
            || self.under_refine(vid)
        {
            return Err(MoveItemRefused::Refused(MoveRefused::WindowOpen));
        }
        let scroll = match arm {
            Arm::Scroll(value) => value,
            Arm::CleanLook => {
                return self.clean_look_step(vid, at, dest, actor);
            }
            Arm::Element(kind) => {
                return self.element_step(vid, at, dest, kind, actor);
            }
            Arm::Stone | Arm::CleanSash | Arm::Plain => {
                return self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
                    let stone = world::character::use_source(items, at)?;
                    let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
                    let proto = protos
                        .get(stone.vnum)
                        .ok_or(MoveRefused::UnknownVnum(stone.vnum))?;
                    world::character::use_checks(&stone, proto, rules, gear.points)?;
                    if matches!(arm, Arm::CleanSash) {
                        return world::character::clean_sash_attr(
                            items,
                            (&stone, proto),
                            dest,
                            protos,
                        );
                    }
                    world::character::put_stone(items, (&stone, proto), dest, protos)
                });
            }
        };
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let used = world::character::use_source(items, at)?;
            let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
            let proto = protos
                .get(used.vnum)
                .ok_or(MoveRefused::UnknownVnum(used.vnum))?;
            world::character::use_checks(&used, proto, rules, gear.points)?;
            Ok(MoveDone {
                kind: MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            })
        })?;
        let window = self.scroll_window(vid, (at, dest), scroll, actor);
        moved.records.extend(window);
        Ok(moved)
    }

    /// The `USE_DETACHMENT` arm with `VALUE0` 8 (`CleanTransmutation`,
    /// `G/char.cpp:10746-10768`): the scroll at `at` cleans the transmutation of the item at
    /// `dest`. A held target chats the locked line; the world reports it through
    /// [`MoveRefused::Locked`], which no other arm of the use-on-an-item path raises.
    fn clean_look_step(
        &mut self,
        vid: Vid,
        at: ItemPos,
        dest: ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let sash = self
            .sash_held
            .values()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let looks = self
            .changelook_held
            .values()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        match self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let stone = world::character::use_source(items, at)?;
            let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
            let proto = protos
                .get(stone.vnum)
                .ok_or(MoveRefused::UnknownVnum(stone.vnum))?;
            world::character::use_checks(&stone, proto, rules, gear.points)?;
            world::character::clean_transmutation(items, (&stone, proto), dest, protos, |id| {
                sash.contains(&Some(id)) || looks.contains(&Some(id))
            })
        }) {
            Err(MoveItemRefused::Refused(MoveRefused::Locked)) => {
                let character = self
                    .characters
                    .find_by_vid(vid)
                    .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
                let pid = character.player_id();
                let mut moved = MovedItems::new(
                    pid,
                    vid.raw(),
                    MoveDone {
                        kind: MoveKind::Used,
                        records: Vec::new(),
                        changes: Vec::new(),
                    },
                    actor,
                    &self.protos,
                    &self.locale,
                );
                moved.records.push(self.refine_line(
                    actor,
                    CHAT_TYPE_INFO,
                    world::character::CLEAN_LOCKED_NOTICE.as_bytes(),
                ));
                Ok(moved)
            }
            answered => answered,
        }
    }

    /// The arm the item at `at` runs; an item with no proto runs the plain use, which
    /// refuses it.
    /// The `USE_ELEMENT_*` arms of `UseItemEx` (`G/char_item.cpp:3663-3779`): the
    /// element scroll at `at` stands its window on the item at `dest`. The use's checks
    /// run first, as the scroll arm's do; the window or the refusing line rides `moved`.
    fn element_step(
        &mut self,
        vid: Vid,
        at: ItemPos,
        dest: ItemPos,
        kind: i32,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let mut moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, protos| {
            let used = world::character::use_source(items, at)?;
            let gear = gear.ok_or(MoveRefused::NotPorted(Unported::Equipment))?;
            let proto = protos
                .get(used.vnum)
                .ok_or(MoveRefused::UnknownVnum(used.vnum))?;
            world::character::use_checks(&used, proto, rules, gear.points)?;
            Ok(MoveDone {
                kind: MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            })
        })?;
        let window = self.element_window(vid, at, dest, kind, actor);
        moved.records.extend(window);
        Ok(moved)
    }

    fn use_arm(&self, vid: Vid, at: ItemPos) -> Arm {
        let proto = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.items().item_at(at))
            .and_then(|item| self.protos.get(item.vnum));
        match proto {
            Some(proto) if proto.item_type == ITEM_METIN => Arm::Stone,
            Some(proto)
                if proto.item_type == ITEM_USE
                    && proto.sub_type == USE_TUNING
                    && proto.values[0] != CL_CLEAN_ATTR_VALUE0 =>
            {
                Arm::Scroll(proto.values[0])
            }
            Some(proto)
                if proto.item_type == ITEM_USE
                    && proto.sub_type == USE_DETACHMENT
                    && proto.values[0] == SASH_CLEAN_ATTR_VALUE0 =>
            {
                Arm::CleanSash
            }
            Some(proto)
                if proto.item_type == ITEM_USE
                    && proto.sub_type == USE_DETACHMENT
                    && proto.values[0] == CL_CLEAN_ATTR_VALUE0 =>
            {
                Arm::CleanLook
            }
            Some(proto)
                if proto.item_type == ITEM_USE
                    && (proto.sub_type == USE_ELEMENT_UPGRADE
                        || proto.sub_type == USE_ELEMENT_DOWNGRADE
                        || proto.sub_type == USE_ELEMENT_CHANGE) =>
            {
                Arm::Element(proto.sub_type)
            }
            _ => Arm::Plain,
        }
    }

    /// The `USE_TUNING` arm of `UseItemEx` (`G/char_item.cpp:5959-6013`) and
    /// `CHARACTER::RefineItem` (`:1970-1995`) once the use's checks passed: the records the
    /// scroll at `at`, of `VALUE0` `value`, sends for the item at `dest`.
    fn scroll_window(
        &mut self,
        vid: Vid,
        (at, dest): (ItemPos, ItemPos),
        value: i32,
        actor: Mover,
    ) -> Vec<Vec<u8>> {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return Vec::new();
        };
        let items = character.items();
        let Some(scroll) = items.item_at(at) else {
            return Vec::new();
        };
        let scroll_vnum = scroll.vnum;
        let Some(target) = world::character::use_target(items, dest) else {
            return Vec::new();
        };
        if PLUS_THREE_STONES.contains(&target.vnum) {
            return vec![self.refine_line(actor, CHAT_TYPE_INFO, PLUS_THREE_NOTICE)];
        }
        if PLUS_FOUR_STONES.contains(&target.vnum) && scroll_vnum != STONE_SCROLL {
            return vec![self.refine_line(actor, CHAT_TYPE_INFO, PLUS_FOUR_NOTICE)];
        }
        let refine_set = self
            .protos
            .get(target.vnum)
            .map_or(0, |proto| proto.refine_set);
        let refine_type = match value {
            MUSIN_SCROLL => REFINE_TYPE_MUSIN,
            HYUNIRON_CHN => REFINE_TYPE_HYUNIRON,
            BDRAGON_SCROLL if refine_set != BDRAGON_SET => return Vec::new(),
            BDRAGON_SCROLL => REFINE_TYPE_BDRAGON,
            _ if refine_set == NO_SCROLL_SET => return Vec::new(),
            _ => REFINE_TYPE_SCROLL,
        };
        let inventory = EWindows::Inventory as u8;
        if at.window_type != inventory || target.pos.window_type != inventory {
            return Vec::new();
        }
        self.refine_information(vid, target.pos.cell, refine_type, Some(at.cell), actor)
    }

    /// `CHARACTER::DoRefineWithScroll` (`G/char_item.cpp:1595-1862`) on the item at `at`,
    /// adding what it does to `moved`.
    pub(super) fn do_refine_with_scroll(
        &mut self,
        vid: Vid,
        at: ItemPos,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        // `G/char_item.cpp:1600-1606`, first; the popup chats.
        if self.is_secured(vid) {
            moved.records.push(self.protected_popup(actor));
            return;
        }
        match self.scroll_plan(vid, at, moved.owner_id) {
            Ok(plan) => self.apply_refine(vid, at, &plan, actor, moved),
            Err(Some(notice)) => {
                moved
                    .records
                    .push(self.refine_line(actor, CHAT_TYPE_INFO, notice));
            }
            Err(None) => {}
        }
    }

    /// `DoRefineWithScroll`'s gates, in legacy's order: the refine it may run, or the line
    /// that refuses it (`None` for a silent refusal).
    fn scroll_plan(
        &self,
        vid: Vid,
        at: ItemPos,
        pid: u32,
    ) -> Result<RefinePlan, Option<&'static [u8]>> {
        if !self.may_refine(vid, pid) {
            return Err(None);
        }
        let character = self.characters.find_by_vid(vid).map_err(|_| None)?;
        let gold = character.gold();
        let old = character.items().item_at(at).cloned().ok_or(None)?;
        let proto = self.protos.get(old.vnum).cloned().ok_or(None)?;
        let recipe = *self
            .refine_recipes
            .get(u32::from(proto.refine_set))
            .ok_or(None)?;
        let cell = self.refine_scroll.get(&vid).copied().ok_or(None)?;
        let scroll = character
            .items()
            .item_at(ItemPos::new(EWindows::Inventory as u8, cell))
            .cloned()
            .ok_or(None)?;
        let scroll_proto = self.protos.get(scroll.vnum).cloned().ok_or(None)?;
        if scroll_proto.item_type != ITEM_USE || scroll_proto.sub_type != USE_TUNING {
            return Err(None);
        }
        if scroll.vnum == old.vnum {
            return Err(None);
        }
        let result_vnum = proto.refined_vnum;
        if result_vnum == 0 {
            return Err(Some(NO_RESULT_NOTICE));
        }
        let value = scroll_proto.values[0];
        let level = refine_level(&proto);
        let wrong_level = match value {
            MUSIN_SCROLL => level >= 4,
            MEMO_SCROLL => level != scroll_proto.values[1],
            BDRAGON_SCROLL => proto.item_type != ITEM_METIN || level != 4,
            _ => false,
        };
        if wrong_level {
            return Err(Some(if value == MEMO_SCROLL {
                MEMO_LEVEL_NOTICE
            } else {
                SCROLL_LEVEL_NOTICE
            }));
        }
        let Some(made) = self.protos.get(result_vnum).cloned() else {
            tracing::error!("DoRefineWithScroll NOT GET ITEM PROTO {result_vnum}");
            return Err(Some(NOT_REFINABLE_NOTICE));
        };
        let fee = u64::try_from(recipe.cost).map_err(|_| Some(REFINE_GOLD_NOTICE))?;
        if gold < fee {
            return Err(Some(REFINE_GOLD_NOTICE));
        }
        let materials: Vec<(u32, i32)> = recipe
            .materials
            .iter()
            .take(usize::from(recipe.material_count))
            .map(|material| (material.vnum, material.count))
            .collect();
        if materials
            .iter()
            .any(|&(vnum, count)| self.count_specify_item(vid, vnum) < count)
        {
            return Err(Some(REFINE_MATERIAL_NOTICE));
        }
        let step = usize::try_from(level.clamp(0, 8)).unwrap_or(0);
        let prob = match value {
            YONGSIN_SCROLL => YONGSIN_PROB[step],
            YAGONG_SCROLL => YAGONG_PROB[step],
            MUSIN_SCROLL | MEMO_SCROLL => 100,
            BDRAGON_SCROLL => 80,
            _ => recipe.prob,
        };
        // A magic iron's failure skips the swap back and takes the last arm, which keeps the
        // item (`bDestroyWhenFail`, `:1736-1737`, `:1853-1858`).
        let from = (value != HYUNIRON_CHN)
            .then(|| self.refine_from(old.vnum))
            .flatten();
        if !self.refined_fits(vid, at, &made)
            || from.is_some_and(|from| !self.refined_fits(vid, at, from))
        {
            return Err(None);
        }
        let fail = from.map_or(FailOutcome::Keep, |from| FailOutcome::Become(from.vnum));
        Ok(RefinePlan {
            old,
            proto,
            made,
            prob,
            fee,
            gold,
            materials,
            money_only: false,
            fail,
            scroll: Some((scroll, scroll_proto)),
        })
    }

    /// `ITEM_MANAGER::GetRefineFromVnum` (`G/item_manager.cpp:83-89`, `:1909-1916`): the
    /// first proto, in vnum order, that refines into `vnum`.
    fn refine_from(&self, vnum: u32) -> Option<&ItemProto> {
        self.protos
            .rows()
            .iter()
            .find(|proto| proto.refined_vnum == vnum)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use db::items::RowChange;
    use gamedata::item_proto::ItemProto;
    use gamedata::records::RefineTableRecord;
    use gamedata::refine::RefineRecipes;
    use world::item::{Item, ItemIdRange};

    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};

    const fn mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
            dead: false,
            alignment: 0,
            view_guild: (0, 0),
            premium: (0, 0),
        }
    }

    /// A world where player 1 holds sword 11 in cell 0 and one `scroll` in cell 3, with 1000
    /// gold, and sword 11's recipe (set 2) refines at 0%.
    fn a_failing_refine(scroll: u32) -> GameState {
        let clock = TestClock::default();
        let recipe = RefineTableRecord {
            id: 2,
            cost: 600,
            prob: 0,
            ..RefineTableRecord::default()
        };
        let mut world =
            a_world(&clock).with_refine_recipes(Arc::new(RefineRecipes::from_records([recipe])));
        world
            .install_item_ids(ItemIdRange::new(100, 1_000_000, 100).expect("a usable range"))
            .expect("the first install");
        let mut sword = Item::new(7, 11);
        sword.size =
            u8::try_from(world.protos().get(11).expect("sword 11").size).expect("a small size");
        let blessing = Item::new(8, scroll);
        let inventory = EWindows::Inventory as u8;
        let items = [
            (ItemPos::new(inventory, 0), sword),
            (ItemPos::new(inventory, 3), blessing),
        ];
        let (_inbox, _) = enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, &items);
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("player 1")
            .set_gold(1000);
        world
    }

    /// Use the scroll on the sword, then answer its window with `refine_type`.
    fn refine_with_scroll(world: &mut GameState, refine_type: u8) -> MovedItems {
        let vid = Vid::new(1);
        let inventory = EWindows::Inventory as u8;
        let used = world
            .use_item_to_item(
                vid,
                ItemPos::new(inventory, 3),
                ItemPos::new(inventory, 0),
                mover(),
            )
            .expect("player 1");
        assert_eq!(used.records.len(), 1, "the window opens");
        assert_eq!(used.records[0][1], refine_type, "the window's type");
        world
            .refine(vid, 0, refine_type, mover())
            .expect("player 1")
    }

    #[test]
    fn arena_limited_items_match_the_complete_legacy_list() {
        for vnum in 50_801..=50_826 {
            assert!(arena_limited_item(vnum));
        }
        for vnum in [50_020, 50_021, 50_022, 71_044, 71_055] {
            assert!(arena_limited_item(vnum));
        }
        for vnum in [
            50_019, 50_023, 50_800, 50_827, 71_043, 71_045, 71_054, 71_056,
        ] {
            assert!(!arena_limited_item(vnum));
        }
    }

    #[test]
    fn arena_potion_rules_pin_flags_whitelists_and_the_unlimited_boundary() {
        for (no_delay, allowed) in [
            (false, vec![27_001, 27_002, 27_003, 27_004, 27_005, 27_006]),
            (true, vec![70_020, 71_018, 71_019, 71_020]),
        ] {
            for vnum in allowed {
                assert_eq!(arena_potion_refusal(vnum, no_delay, 0, 9_999, 1), None);
                for left in [0, -1] {
                    assert_eq!(
                        arena_potion_refusal(vnum, no_delay, 0, 9_999, left),
                        Some(&b"[LS;122]"[..])
                    );
                    for limit in [10_000, 10_001] {
                        assert_eq!(arena_potion_refusal(vnum, no_delay, 0, limit, left), None);
                    }
                }
                assert_eq!(arena_potion_refusal(vnum, no_delay, -1, 0, 1), None);
                assert_eq!(
                    arena_potion_refusal(vnum, no_delay, 1, 10_001, 1),
                    Some(&b"[LS;403]"[..])
                );
            }
        }
        for (vnum, no_delay) in [
            (27_000, false),
            (27_007, false),
            (27_001, true),
            (71_018, false),
            (70_019, true),
            (71_021, true),
        ] {
            assert_eq!(
                arena_potion_refusal(vnum, no_delay, 0, 10_001, 1),
                Some(&b"[LS;403]"[..])
            );
        }
    }

    fn arena_item_world(vnum: u32, item_type: i32, sub_type: i32, values: [i32; 6]) -> GameState {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut proto = ItemProto::for_category_rule(vnum, item_type, sub_type);
        proto.values = values;
        world.protos = Arc::new(gamedata::item_proto::ItemProtos::from_rows(vec![proto]));
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let mut item = Item::new(7, vnum);
        item.count = 2;
        let (_inbox, _) = enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, &[(at, item)]);
        world.set_arena_potion_count(Vid::new(1), 2);
        world
    }

    fn mark_arena(world: &mut GameState) {
        world.load_arenas(&[quest::host::Arena {
            map_index: PLACE.1,
            start_a: (1, 1),
            start_b: (2, 2),
        }]);
    }

    #[test]
    fn shared_gate_covers_quest_items_visitors_empty_cells_and_item_targets() {
        let mut world = arena_item_world(71_044, gamedata::item_kind::ITEM_QUEST, 0, [0; 6]);
        let vid = Vid::new(1);
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        assert!(world.arena_item_use_allowed(vid, at));
        mark_arena(&mut world);
        assert!(!world.in_arena(vid));
        assert!(!world.arena_item_use_allowed(vid, at));
        assert!(world.arena_item_use_allowed(vid, ItemPos::new(EWindows::Inventory as u8, 1)));
        let step = world
            .use_item(vid, at, mover(), &crate::game_loop_messages::QuestAccount::default())
            .expect("visitor");
        assert!(step.changes.is_empty());
        let mut targeted = arena_item_world(71_044, ITEM_USE, USE_TUNING, [0; 6]);
        mark_arena(&mut targeted);
        let step = targeted
            .use_item_to_item(vid, at, at, mover())
            .expect("visitor");
        assert!(step.changes.is_empty());
        assert!(step.records.is_empty());
    }

    #[test]
    fn quest_seals_refuse_bound_duelists_but_not_plain_map_visitors() {
        let mut world = arena_item_world(50_051, gamedata::item_kind::ITEM_QUEST, 0, [0; 6]);
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let vid = Vid::new(1);
        mark_arena(&mut world);
        assert!(!world.in_arena(vid));
        let other = Vid::new(2);
        let _other = crate::game_state::fixtures::enter(&mut world, other.raw(), (6, 5), 10_000);
        world.start_arena(
            ((0, u32::try_from(PLACE.1).unwrap()), 0),
            [vid, other],
            3,
            5,
        );
        world.run_arena_ticks(250);
        assert!(world.in_arena(vid));
        let result = world
            .use_item(vid, at, mover(), &crate::game_loop_messages::QuestAccount::default())
            .expect("seal stays");
        assert!(result.changes.is_empty());
        assert_eq!(
            world
                .characters
                .find_by_vid(vid)
                .unwrap()
                .items()
                .item_at(at)
                .unwrap()
                .count,
            2
        );
    }

    #[test]
    fn map_potion_gate_rejects_unlisted_vnums_without_requiring_membership() {
        let mut world = arena_item_world(
            27_101,
            ITEM_USE,
            gamedata::item_kind::USE_POTION_NODELAY,
            [100, 0, 0, 0, 0, 0],
        );
        let vid = Vid::new(1);
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        assert!(world.arena_item_use_allowed(vid, at));
        mark_arena(&mut world);
        assert!(!world.arena_item_use_allowed(vid, at));
        assert!(world.arena_item_use_allowed(Vid::new(999), at));
        let mut plain = arena_item_world(11, gamedata::item_kind::ITEM_WEAPON, 0, [0; 6]);
        mark_arena(&mut plain);
        assert!(plain.arena_item_use_allowed(vid, at));
    }

    #[test]
    fn nonpotion_or_unconsumed_item_steps_do_not_spend_the_potion_count() {
        let mut world = arena_item_world(27_001, ITEM_USE, gamedata::item_kind::USE_POTION, [0; 6]);
        let vid = Vid::new(1);
        let mut moved = MovedItems::new(
            1,
            vid.raw(),
            MoveDone {
                kind: MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            },
            mover(),
            &world.protos,
            &world.locale,
        );
        world.arena_consumed_potion(vid, 27_001, gamedata::item_kind::USE_POTION, &moved);
        assert_eq!(world.arena_potion_count(vid), 2);
        moved.changes.push(RowChange::Destroyed { id: 7 });
        world.arena_consumed_potion(vid, 27_001, USE_TUNING, &moved);
        assert_eq!(world.arena_potion_count(vid), 2);
    }

    #[test]
    fn standard_zero_value_potion_is_consumed_but_instant_zero_value_is_not() {
        for (sub_type, accepted) in [
            (gamedata::item_kind::USE_POTION, true),
            (gamedata::item_kind::USE_POTION_NODELAY, false),
        ] {
            let mut world = arena_item_world(27_001, ITEM_USE, sub_type, [0; 6]);
            let vid = Vid::new(1);
            let at = ItemPos::new(EWindows::Inventory as u8, 0);
            let step = world.use_item(vid, at, mover(), &crate::game_loop_messages::QuestAccount::default());
            assert_eq!(step.is_ok(), accepted);
            assert_eq!(world.arena_potion_count(vid), if accepted { 1 } else { 2 });
        }
    }

    #[test]
    fn accepted_potions_decrement_and_refused_or_partial_uses_do_not() {
        for (vnum, sub_type) in [
            (27_001, gamedata::item_kind::USE_POTION),
            (71_018, gamedata::item_kind::USE_POTION_NODELAY),
        ] {
            for arena in [false, true] {
                let mut world = arena_item_world(vnum, ITEM_USE, sub_type, [100, 0, 0, 0, 0, 0]);
                if arena {
                    mark_arena(&mut world);
                }
                let vid = Vid::new(1);
                let at = ItemPos::new(EWindows::Inventory as u8, 0);
                assert!(world
                    .use_item(vid, at, mover(), &crate::game_loop_messages::QuestAccount::default())
                    .is_err());
                assert_eq!(world.arena_potion_count(vid), 2);
                world
                    .pc_points_mut(vid.raw())
                    .unwrap()
                    .point_change(common::point_slot::POINT_HP, -300, false, false)
                    .unwrap();
                world
                    .use_item(vid, at, mover(), &crate::game_loop_messages::QuestAccount::default())
                    .expect("accepted");
                assert_eq!(world.arena_potion_count(vid), 1);
                world.set_arena_potion_count(vid, 0);
                let step = world
                    .use_item(vid, at, mover(), &crate::game_loop_messages::QuestAccount::default())
                    .expect("second");
                assert_eq!(world.arena_potion_count(vid), if arena { 0 } else { -1 });
                assert_eq!(step.changes.is_empty(), arena);
            }
        }
        let mut world = arena_item_world(
            27_001,
            ITEM_USE,
            gamedata::item_kind::USE_POTION,
            [100, 50, 0, 0, 0, 0],
        );
        mark_arena(&mut world);
        let vid = Vid::new(1);
        world
            .pc_points_mut(vid.raw())
            .unwrap()
            .point_change(common::point_slot::POINT_SP, -100, false, false)
            .unwrap();
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let step = world
            .use_item(vid, at, mover(), &crate::game_loop_messages::QuestAccount::default())
            .expect("partial");
        assert_eq!(step.kind, MoveKind::Declined);
        assert!(step.changes.is_empty());
        assert_eq!(world.arena_potion_count(vid), 2);
    }

    /// A blessing scroll's failure swaps the sword for the one it was refined from, keeping
    /// nothing else of the draw; the scroll and the fee are spent.
    #[test]
    fn a_failed_scroll_refine_steps_the_item_back() {
        let mut world = a_failing_refine(25_040);
        let moved = refine_with_scroll(&mut world, REFINE_TYPE_SCROLL);
        assert!(matches!(moved.changes[0], RowChange::Destroyed { id: 8 }));
        assert!(matches!(moved.changes[1], RowChange::Destroyed { id: 7 }));
        let RowChange::Created(row) = &moved.changes[2] else {
            panic!("the sword it came from is made: {:?}", moved.changes);
        };
        assert_eq!((row.vnum, row.pos), (10, 0));
        assert_eq!(moved.gold, Some(-600));
    }

    /// A magic iron's failure keeps the sword; the iron and the fee are spent.
    #[test]
    fn a_failed_magic_iron_keeps_the_item() {
        let mut world = a_failing_refine(25_041);
        let moved = refine_with_scroll(&mut world, REFINE_TYPE_HYUNIRON);
        assert_eq!(moved.changes.len(), 1, "{:?}", moved.changes);
        assert!(matches!(moved.changes[0], RowChange::Destroyed { id: 8 }));
        assert_eq!(moved.gold, Some(-600));
    }

    fn named(name: &[u8]) -> ItemProto {
        let mut proto = ItemProto::for_category_rule(10, 1, 0);
        proto.name = name.to_vec();
        proto
    }

    /// The level is the number after the last `+`, read as `strtol` reads it.
    #[test]
    fn the_refine_level_is_the_number_after_the_last_plus() {
        assert_eq!(refine_level(&named(b"\xb5\xb5+0")), 0);
        assert_eq!(refine_level(&named(b"\xb5\xb5+7")), 7);
        assert_eq!(refine_level(&named(b"a+1+12x")), 12);
        assert_eq!(refine_level(&named(b"plain")), 0);
        assert_eq!(refine_level(&named(b"a+ -3")), -3);
        assert_eq!(refine_level(&named(b"a+")), 0);
    }
}
