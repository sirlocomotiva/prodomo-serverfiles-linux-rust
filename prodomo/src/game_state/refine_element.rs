//! The element refine (`ENABLE_REFINE_ELEMENT`): an element scroll used on a +7 weapon
//! stands the window (`RefineElementInformation`, `G/char_item.cpp:10452-10472`), and
//! `CG_REFINE_ELEMENT` (`CInputMain::RefineElement`, `G/input_main.cpp:3233-3249`) runs
//! `DoRefineElement` (`G/char_item.cpp:10244-10424`) or closes it.
//!
//! The window is the scroll's cell, the weapon's cell and the kind (upgrade, downgrade,
//! change); every request clears it first. The upgrade needs the scroll's `VALUE0` element
//! (1 to 6), the weapon below element +3, and no other element on it; the downgrade needs
//! an elemented weapon; the change needs the request's own element (1 to 6). The fee is
//! 3M for an upgrade and 10M else; the draw is `number(1, 100)` against 100, so it always
//! lands. The answer is one `GC_REFINE_ELEMENT`: the kind echoed for the window, 10/12/13
//! for a landed upgrade/downgrade/change, 11 for a failed draw.
//!
//! The element packs as `%d%d%02d%02d%d%02d`: the kind, the plus, the bonus, the attack,
//! and the last two raises (`item.h:97-102` reads them back). The upgrade raises the plus
//! and both values by `number(1, 8)` and `number(2, 12)`; the downgrade at +1 clears the
//! element, else it lowers the plus and takes the values down by their last raises,
//! clamping those to the lowered values minus 1; the change swaps the kind. The scroll
//! leaves and the gold leaves before the draw, so a failed draw keeps the spends.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::item_slots::{usable_inventory_cells, EWindows};
use common::vid::Vid;
use gamedata::item_kind::{
    ITEM_USE, ITEM_WEAPON, USE_ELEMENT_CHANGE, USE_ELEMENT_DOWNGRADE, USE_ELEMENT_UPGRADE,
};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_vid::GcRefineElement;
use protocol::item_pos::ItemPos;

use super::GameState;
use crate::item_move::{gold_delta, MoveItemRefused, MovedItems, Mover};

/// `REFINE_ELEMENT_TYPE_UPGRADE` (`C/length.h:1162`): the window kind an upgrade scroll
/// stands, and the request kind that runs it.
pub const REFINE_ELEMENT_UPGRADE: u8 = 0;
/// `REFINE_ELEMENT_TYPE_DOWNGRADE` (`C/length.h:1163`).
pub const REFINE_ELEMENT_DOWNGRADE: u8 = 1;
/// `REFINE_ELEMENT_TYPE_CHANGE` (`C/length.h:1164`).
pub const REFINE_ELEMENT_CHANGE: u8 = 2;
/// `REFINE_ELEMENT_TYPE_UPGRADE_SUCCES` (`C/length.h:1165`).
pub const REFINE_ELEMENT_UPGRADE_DONE: u8 = 10;
/// `REFINE_ELEMENT_TYPE_UPGRADE_FAIL` (`C/length.h:1166`): the answer to a failed draw,
/// whatever the kind.
pub const REFINE_ELEMENT_FAILED: u8 = 11;
/// `REFINE_ELEMENT_TYPE_DOWNGRADE_SUCCES` (`C/length.h:1167`).
pub const REFINE_ELEMENT_DOWNGRADE_DONE: u8 = 12;
/// `REFINE_ELEMENT_TYPE_CHANGE_SUCCES` (`C/length.h:1168`).
pub const REFINE_ELEMENT_CHANGE_DONE: u8 = 13;
/// The `bArg` that closes the window (`G/input_main.cpp:3245`).
pub const REFINE_ELEMENT_CLOSE: u8 = 255;

/// `ELEMENT_MIN_REFINE_LEVEL` (`C/length.h:1153`): an upgrade needs a +7 weapon.
pub const ELEMENT_MIN_REFINE_LEVEL: i32 = 7;
/// `REFINE_ELEMENT_MAX` (`C/length.h:1152`): the highest element plus.
pub const REFINE_ELEMENT_MAX_PLUS: u8 = 3;

/// `REFINE_ELEMENT_CATEGORY_MAX` (`C/length.h:1139-1147`): the elements are 1 to 7
/// (electricity, fire, ice, wind, earth, darkness).
pub const REFINE_ELEMENT_CATEGORY_MAX: i32 = 8;
/// `REFINE_ELEMENT_UPGRADE_YANG` (`C/length.h:1154`).
pub const REFINE_ELEMENT_UPGRADE_FEE: u64 = 3_000_000;
/// `REFINE_ELEMENT_DOWNGRADE_YANG` and `REFINE_ELEMENT_CHANGE_YANG`
/// (`C/length.h:1155-1156`).
pub const REFINE_ELEMENT_OTHER_FEE: u64 = 10_000_000;

/// The refine `DoRefineElement` may run: where the scroll and the weapon are, the
/// window kind and the request's element, and the fee and the landed answer.
struct ElementPlan {
    /// The scroll's cell.
    at: ItemPos,
    /// The weapon's cell.
    target: ItemPos,
    /// The window kind: upgrade, downgrade or change.
    request: u8,
    /// The request's element, for a change.
    arg: u8,
    /// The scroll's element, for an upgrade.
    element: u8,
    /// The weapon's unpacked element.
    kind: u8,
    /// Its plus.
    plus: u8,
    /// Its bonus value.
    bonus: u8,
    /// Its attack value.
    attack: u8,
    /// Its last bonus raise.
    last_bonus: u8,
    /// Its last attack raise.
    last_attack: u8,
    /// The fee.
    fee: u64,
    /// The landed answer.
    done: u8,
}

/// The element of a scroll, or `None` when its `VALUE0` names no element.
fn scroll_element(value0: i32) -> Option<u8> {
    u8::try_from(value0)
        .ok()
        .filter(|kind| (1..REFINE_ELEMENT_CATEGORY_MAX).contains(&i32::from(*kind)))
}

/// The six fields of a packed element, in `%d%d%02d%02d%d%02d` order.
fn unpack(element: u32) -> (u8, u8, u8, u8, u8, u8) {
    if element == 0 {
        return (0, 0, 0, 0, 0, 0);
    }
    (
        (element / 100_000_000) as u8,
        (element / 10_000_000 % 10) as u8,
        (element / 100_000 % 100) as u8,
        (element / 1_000 % 100) as u8,
        (element / 100 % 10) as u8,
        (element % 100) as u8,
    )
}

/// Pack the six fields the way the `snprintf` lays them (`%d%d%02d%02d%d%02d`).
fn pack(kind: u8, plus: u8, bonus: u8, attack: u8, last_bonus: u8, last_attack: u8) -> u32 {
    u32::from(kind) * 100_000_000
        + u32::from(plus) * 10_000_000
        + u32::from(bonus) * 100_000
        + u32::from(attack) * 1_000
        + u32::from(last_bonus) * 100
        + u32::from(last_attack)
}

impl GameState {
    /// An element scroll used on an item: the window the `USE_ELEMENT_*` arms of
    /// `UseItemEx` stand (`G/char_item.cpp:3663-3779`). `kind` is the scroll's sub-type;
    /// the records the user under `vid` is sent, the window's echo or the line that
    /// refuses it.
    pub(super) fn element_window(
        &mut self,
        vid: Vid,
        at: ItemPos,
        dest: ItemPos,
        kind: i32,
        actor: Mover,
    ) -> Vec<Vec<u8>> {
        let info = self.element_arms(vid, at, dest, kind, actor);
        let Some((src, tgt, request, _scroll, _weapon)) = info else {
            return self.element_lines.take().unwrap_or_default();
        };
        self.element_information(vid, src, tgt, request, actor)
    }

    /// The lines `element_window` refused with, held while the arms run.
    fn element_push(&mut self, actor: Mover, text: &[u8]) {
        let line = self.refine_line(actor, CHAT_TYPE_INFO, text);
        self.element_lines.get_or_insert_default().push(line);
    }

    /// The three `USE_ELEMENT_*` arms' gates: the scroll's and weapon's cells, the window
    /// kind, and both items. A refusal chats its line and answers `None`.
    fn element_arms(
        &mut self,
        vid: Vid,
        at: ItemPos,
        dest: ItemPos,
        kind: i32,
        actor: Mover,
    ) -> Option<(u16, u16, u8, world::item::Item, world::item::Item)> {
        self.element_lines = None;
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return None;
        };
        if at.window_type != EWindows::Inventory as u8
            || dest.window_type != EWindows::Inventory as u8
        {
            return None;
        }
        let scroll = character.items().item_at(at).cloned()?;
        let weapon = character.items().item_at(dest).cloned()?;
        let armed = if kind == USE_ELEMENT_UPGRADE {
            REFINE_ELEMENT_UPGRADE
        } else if kind == USE_ELEMENT_DOWNGRADE {
            REFINE_ELEMENT_DOWNGRADE
        } else {
            REFINE_ELEMENT_CHANGE
        };
        let scroll_proto = self.protos.get(scroll.vnum).cloned()?;
        let weapon_proto = self.protos.get(weapon.vnum).cloned()?;
        if kind == USE_ELEMENT_UPGRADE && scroll_element(scroll_proto.values[0]).is_none() {
            self.element_push(actor, b"Cannot be used with this item.");
            return None;
        }
        if weapon_proto.item_type != ITEM_WEAPON {
            self.element_push(actor, b"Cannot be used with this item.");
            return None;
        }
        if kind == USE_ELEMENT_UPGRADE {
            if world::character::refine_level(&weapon_proto.name) < ELEMENT_MIN_REFINE_LEVEL {
                self.element_push(
                    actor,
                    b"The elemental enchantment is only available for +7 weapons or higher.",
                );
                return None;
            }
            if unpack(weapon.refine_element).1 == REFINE_ELEMENT_MAX_PLUS {
                self.element_push(
                    actor,
                    b"The maximum elemental enchantment level has already been reached.",
                );
                return None;
            }
            let element = scroll_element(scroll_proto.values[0]).unwrap_or(0);
            if unpack(weapon.refine_element).0 > 0 && unpack(weapon.refine_element).0 != element {
                self.element_push(actor, b"The item already has another element.");
                return None;
            }
            if character.gold() < REFINE_ELEMENT_UPGRADE_FEE {
                self.element_push(actor, b"Not enough Yang.");
                return None;
            }
        } else {
            if unpack(weapon.refine_element).1 == 0 {
                return None;
            }
            if character.gold() < REFINE_ELEMENT_OTHER_FEE {
                self.element_push(actor, b"Not enough Yang.");
                return None;
            }
        }
        if scroll_proto.item_type != ITEM_USE {
            return None;
        }
        Some((at.cell, dest.cell, armed, scroll, weapon))
    }

    /// `CHARACTER::RefineElementInformation` (`G/char_item.cpp:10452-10472`): echo the
    /// window and stand it.
    fn element_information(
        &mut self,
        vid: Vid,
        src: u16,
        dst: u16,
        request: u8,
        actor: Mover,
    ) -> Vec<Vec<u8>> {
        let _ = actor;
        let mut out = Vec::new();
        GcRefineElement {
            src_cell: src,
            dst_cell: dst,
            element_type: request,
        }
        .encode_into(&mut out);
        self.element_windows.insert(vid, (src, dst, request));
        vec![out]
    }

    /// Run one `CG_REFINE_ELEMENT` for the character online under `vid`
    /// (`CInputMain::RefineElement`, `G/input_main.cpp:3233-3249`). A trade, an open
    /// safebox or an open cube chats and clears; 255 clears; anything else runs
    /// `DoRefineElement`.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn refine_element(
        &mut self,
        vid: Vid,
        arg: u8,
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
            world::character::MoveDone {
                kind: world::character::MoveKind::Refined,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        if self.trading.contains_key(&vid)
            || self.safebox_open(vid)
            || self.cubes.contains_key(&vid)
        {
            moved
                .records
                .push(self.refine_line(actor, CHAT_TYPE_INFO, b"Can't do this right now"));
            self.element_windows.remove(&vid);
            return Ok(moved);
        }
        if arg == REFINE_ELEMENT_CLOSE {
            self.element_windows.remove(&vid);
            return Ok(moved);
        }
        self.do_refine_element(vid, arg, actor, &mut moved);
        Ok(moved)
    }

    /// `CHARACTER::DoRefineElement` (`G/char_item.cpp:10244-10424`): the gates, the
    /// spends, the draw, and the landing.
    fn do_refine_element(&mut self, vid: Vid, arg: u8, actor: Mover, moved: &mut MovedItems) {
        let Some((src, dst, request)) = self.element_windows.remove(&vid) else {
            return;
        };
        let Some(plan) = self.element_plan(vid, src, dst, request, arg) else {
            return;
        };
        // The scroll and the gold leave before the draw, but the gold record rides last,
        // after the landed answer.
        let gold = self.take_element_fee(vid, plan.fee, moved);
        self.spend_element_scroll(vid, actor, plan.at, moved);
        if world::character::number(&mut self.dice, 1, 100) > 100 {
            Self::element_send(src, dst, REFINE_ELEMENT_FAILED, moved);
            self.send_element_gold(vid, actor, gold, moved);
            return;
        }
        let packed = self.element_pack(&plan);
        self.set_element(vid, actor, plan.target, packed, moved);
        Self::element_send(src, dst, plan.done, moved);
        self.send_element_gold(vid, actor, gold, moved);
    }

    /// One `GC_REFINE_ELEMENT` folded into `moved`.
    fn element_send(src: u16, dst: u16, kind: u8, moved: &mut MovedItems) {
        let mut out = Vec::new();
        GcRefineElement {
            src_cell: src,
            dst_cell: dst,
            element_type: kind,
        }
        .encode_into(&mut out);
        moved.records.push(out);
    }

    /// `DoRefineElement`'s gates, in legacy's order: the refine it may run, or `None`
    /// for a silent refusal.
    fn element_plan(
        &self,
        vid: Vid,
        src: u16,
        dst: u16,
        request: u8,
        arg: u8,
    ) -> Option<ElementPlan> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let at = ItemPos::new(EWindows::Inventory as u8, src);
        let target = ItemPos::new(EWindows::Inventory as u8, dst);
        let scroll = character.items().item_at(at).cloned()?;
        let weapon = character.items().item_at(target).cloned()?;
        let scroll_proto = self.protos.get(scroll.vnum).cloned()?;
        let weapon_proto = self.protos.get(weapon.vnum).cloned()?;
        if scroll_proto.item_type != ITEM_USE || weapon_proto.item_type != ITEM_WEAPON {
            return None;
        }
        if world::character::refine_level(&weapon_proto.name) < ELEMENT_MIN_REFINE_LEVEL {
            return None;
        }
        let (kind, plus, bonus, attack, last_bonus, last_attack) = unpack(weapon.refine_element);
        let element = scroll_element(scroll_proto.values[0]);
        let (fee, done) = if scroll_proto.sub_type == USE_ELEMENT_UPGRADE {
            if request != REFINE_ELEMENT_UPGRADE {
                return None;
            }
            let element = element?;
            if plus == REFINE_ELEMENT_MAX_PLUS {
                return None;
            }
            if kind > 0 && kind != element {
                return None;
            }
            (REFINE_ELEMENT_UPGRADE_FEE, REFINE_ELEMENT_UPGRADE_DONE)
        } else if scroll_proto.sub_type == USE_ELEMENT_DOWNGRADE {
            if request != REFINE_ELEMENT_DOWNGRADE {
                return None;
            }
            (REFINE_ELEMENT_OTHER_FEE, REFINE_ELEMENT_DOWNGRADE_DONE)
        } else if scroll_proto.sub_type == USE_ELEMENT_CHANGE {
            if request != REFINE_ELEMENT_CHANGE {
                return None;
            }
            if !(1..REFINE_ELEMENT_CATEGORY_MAX).contains(&i32::from(arg)) {
                return None;
            }
            (REFINE_ELEMENT_OTHER_FEE, REFINE_ELEMENT_CHANGE_DONE)
        } else {
            return None;
        };
        if character.gold() < fee {
            return None;
        }
        Some(ElementPlan {
            at,
            target,
            request,
            arg,
            element: element.unwrap_or(0),
            kind,
            plus,
            bonus,
            attack,
            last_bonus,
            last_attack,
            fee,
            done,
        })
    }

    /// `DoRefineElement`'s landing: the packed element the draw lays on the weapon.
    fn element_pack(&mut self, plan: &ElementPlan) -> u32 {
        if plan.request == REFINE_ELEMENT_UPGRADE {
            let raise_bonus =
                u8::try_from(world::character::number(&mut self.dice, 1, 8)).unwrap_or(u8::MAX);
            let raise_attack =
                u8::try_from(world::character::number(&mut self.dice, 2, 12)).unwrap_or(u8::MAX);
            pack(
                plan.element,
                plan.plus + 1,
                plan.bonus.saturating_add(raise_bonus),
                plan.attack.saturating_add(raise_attack),
                raise_bonus,
                raise_attack,
            )
        } else if plan.request == REFINE_ELEMENT_DOWNGRADE {
            if plan.plus == 1 {
                0
            } else {
                let bonus = plan.bonus.saturating_sub(plan.last_bonus);
                let attack = plan.attack.saturating_sub(plan.last_attack);
                pack(
                    plan.kind,
                    plan.plus - 1,
                    bonus,
                    attack,
                    plan.last_bonus.min(bonus.saturating_sub(1)),
                    plan.last_attack.min(attack.saturating_sub(1)),
                )
            }
        } else {
            pack(
                plan.arg,
                plan.plus,
                plan.bonus,
                plan.attack,
                plan.last_bonus,
                plan.last_attack,
            )
        }
    }

    /// Spend one scroll at `at` for an element refine, its records and changes folded
    /// into `moved`.
    fn spend_element_scroll(
        &mut self,
        vid: Vid,
        actor: Mover,
        at: ItemPos,
        moved: &mut MovedItems,
    ) {
        let protos = self.protos.clone();
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let Some(held) = character.items().item_at(at).cloned() else {
            return;
        };
        let usable = usable_inventory_cells(character.inven_point());
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let done =
            world::character::remove_specify_item(items, slots, &protos, usable, (held.vnum, 1));
        let packed = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            world::character::MoveDone {
                kind: world::character::MoveKind::Removed,
                records: done.records,
                changes: done.changes,
            },
            actor,
            &protos,
            &self.locale,
        );
        moved.records.extend(packed.records);
        moved.changes.extend(packed.changes);
    }

    /// Spend the element fee for the character under `vid`.
    /// Take the element fee, folding the delta into `moved` and answering the gold the
    /// character holds now.
    fn take_element_fee(&mut self, vid: Vid, fee: u64, moved: &mut MovedItems) -> Option<u64> {
        let character = self.characters.find_by_vid_mut(vid).ok()?;
        let before = character.gold();
        let gold = before.saturating_sub(fee);
        character.set_gold(gold);
        moved.gold = Some(gold_delta(before, gold));
        Some(gold)
    }

    /// The gold record `take_element_fee` withheld, folded into `moved` now.
    fn send_element_gold(&self, vid: Vid, actor: Mover, gold: Option<u64>, moved: &mut MovedItems) {
        let Some(gold) = gold else {
            return;
        };
        let packed = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            world::character::MoveDone {
                kind: world::character::MoveKind::Refined,
                records: vec![world::character::MoveRecord::Gold {
                    amount: 0,
                    value: gold,
                }],
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        moved.records.extend(packed.records);
    }

    /// Lay `packed` on the weapon at `target`, its update record and row folded into
    /// `moved`.
    fn set_element(
        &mut self,
        vid: Vid,
        actor: Mover,
        target: ItemPos,
        packed: u32,
        moved: &mut MovedItems,
    ) {
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let Some(held) = character.items().item_at(target).cloned() else {
            return;
        };
        let (items, _, _) = character.items_points_and_quickslots_mut();
        if items.set_refine_element(held.id, packed).is_err() {
            return;
        }
        let Some(stored) = items.item(held.id) else {
            return;
        };
        let done = world::character::MoveDone {
            kind: world::character::MoveKind::Refined,
            records: vec![world::character::MoveRecord::Item(
                world::character::ItemRecord::Update(stored.gc_item_update()),
            )],
            changes: vec![world::character::ItemChange::Elemented {
                id: held.id,
                element: packed,
            }],
        };
        let packed = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            done,
            actor,
            &self.protos,
            &self.locale,
        );
        moved.records.extend(packed.records);
        moved.changes.extend(packed.changes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pack` lays the fields where `RefineElementTransport::SetRefineElement` lays
    /// them, and `unpack` reads them back.
    #[test]
    fn the_packing_maps_each_field_to_its_place() {
        assert_eq!(pack(1, 2, 3, 4, 5, 6), 120_304_506);
        assert_eq!(unpack(120_304_506), (1, 2, 3, 4, 5, 6));
        assert_eq!(unpack(0), (0, 0, 0, 0, 0, 0));
    }

    /// `number` over `1..=8` and `2..=12` stays in a byte, so the raises land whole.
    #[test]
    fn the_raises_fit_a_byte() {
        for raise in [i32::from(u8::MIN), 1, 12, i32::from(u8::MAX)] {
            assert!(u8::try_from(raise).is_ok(), "{raise}");
        }
    }

    /// The scroll's `VALUE0` is the element itself (`G/char_item.cpp:10316`).
    #[test]
    fn scroll_value_zero_names_the_element() {
        assert_eq!(scroll_element(1), Some(1));
        assert_eq!(scroll_element(6), Some(6));
        assert_eq!(scroll_element(0), None);
        assert_eq!(scroll_element(8), None);
    }
}
