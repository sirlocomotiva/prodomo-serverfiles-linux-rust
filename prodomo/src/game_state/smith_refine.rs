//! The smith's refine: the window a gift to a smith opens (`CHARACTER::RefineInformation`,
//! `G/char_item.cpp:1865-1968`), and the `CG_REFINE` request that answers it
//! (`CInputMain::Refine`, `G/input_main.cpp:3316-3391` → `CHARACTER::DoRefine`,
//! `G/char_item.cpp:1421-1585`, or `DoRefineWithScroll`, `:1595-1862`, for the window a
//! scroll used on an item opens; see `use_to_item`).
//!
//! A window puts the character under refine (`SetRefineMode`), and every request ends it
//! (`ClearRefineMode`). A request that names an item stamps the refine time, which `IsHack`
//! reads. The window also remembers the scroll's cell (`m_iRefineAdditionalCell`), which
//! `ClearRefineMode` leaves, so a later scroll request reads the last window's cell.
//!
//! What is not here:
//!
//! - A smith of a guild (`GetRefineGuild`, `G/char.cpp:8169-8212`) stands only on guild land,
//!   which the Rewrite does not build, so the fee is the recipe's cost, the draw is not
//!   lowered for a guild, and no fee is deposited.
//! - The lock gates the normal and the scroll refines at legacy's places, with the
//!   popup it chats. The personal shop, the cube, the aura and the dragon soul refine
//!   window are not in the Rewrite, so none of them refuses.
//! - The money, item and refine logs are `sys.log`'s, which owns no sink.
//!
//! Two Divergences: a gift from a custom bank (cell 290 and up) opens no window, where legacy
//! truncates the cell to the record's `BYTE` and names another item; and a refined item that
//! would not fit its cell is refused before anything changes, where legacy's `AddToCharacter`
//! lays it over its neighbours.
//!
//! `G/` = `server/server/game/`.

use std::sync::Arc;

use common::item_slots::{usable_inventory_cells, EWindows, INVENTORY_MAX_NUM};
use common::vid::Vid;
use gamedata::item_kind::{ITEM_USE, ITEM_WEAPON, LIMIT_LEVEL, USE_TUNING};
use gamedata::item_proto::ItemProto;
use gamedata::refine::RefineRecipes;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use protocol::gc_refine::{GcRefineInformation, RefineMaterial, REFINE_MATERIAL_SLOTS};
use protocol::item_pos::ItemPos;
use world::character::{MoveDone, MoveKind, MoveRecord};

use super::GameState;
use crate::item_move::{belt_grade, gold_delta, MoveItemRefused, MovedItems, Mover};

/// `REFINE_TYPE_NORMAL`: a town smith's refine.
pub const REFINE_TYPE_NORMAL: u8 = 0;
/// `REFINE_TYPE_MONEY_ONLY`: a devil-tower smith's refine.
pub const REFINE_TYPE_MONEY_ONLY: u8 = 4;
/// `REFINE_TYPE_SCROLL`: a blessing scroll's refine.
pub const REFINE_TYPE_SCROLL: u8 = 2;
/// `REFINE_TYPE_HYUNIRON`: a magic iron's refine.
pub const REFINE_TYPE_HYUNIRON: u8 = 3;
/// `REFINE_TYPE_MUSIN`: a war god's blessing's refine.
pub const REFINE_TYPE_MUSIN: u8 = 5;
/// `REFINE_TYPE_BDRAGON`: a black dragon scroll's refine.
pub const REFINE_TYPE_BDRAGON: u8 = 6;
/// The type a closed window sends (`G/input_main.cpp:3327-3332`).
pub const REFINE_TYPE_CANCEL: u8 = 255;

/// A request while a trade, the safebox or a shop is open (`G/input_main.cpp:3322`).
pub const REFINE_BUSY_NOTICE: &[u8] = b"[LS;1014]";
/// A devil-tower refine without the quest's leave (`G/char_item.cpp:1882`).
pub const DEVIL_TOWER_REFINE_NOTICE: &[u8] = b"[LS;1067]";
/// An item that cannot be refined further, or has no recipe.
pub const NOT_REFINABLE_NOTICE: &[u8] = b"[LS;1002]";
/// A refine scroll given to a smith (`G/char_item.cpp:1904-1907`).
pub const SCROLL_AT_SMITH_NOTICE: &[u8] = b"[LS;1077]";
/// The free refine of `main_quest_lv7` on an item it does not cover.
pub const REFINE_CHANCE_NOTICE: &[u8] = b"[LS;1003]";
/// `DoRefine` on an item with no refined vnum (`G/char_item.cpp:1475`).
pub const NO_RESULT_NOTICE: &[u8] = b"[LS;991]";
/// Too little gold for the fee (`G/char_item.cpp:1498`).
pub const REFINE_GOLD_NOTICE: &[u8] = b"[LS;67]";
/// Too few of a material (`G/char_item.cpp:1515`).
pub const REFINE_MATERIAL_NOTICE: &[u8] = b"[LS;1035]";
/// `NotifyRefineSuccess`'s command (`G/char_item.cpp:1393`).
pub const REFINE_SUCCEEDED: &[u8] = b"RefineSuceeded";
/// `NotifyRefineFail`'s command (`G/char_item.cpp:1403`).
pub const REFINE_FAILED: &[u8] = b"RefineFailed";

/// The devil tower's leave to refine once with money only.
const DEVIL_TOWER_FLAG: &[u8] = b"deviltower_zone.can_refine";
/// The free refines `main_quest_lv7` grants.
const REFINE_CHANCE_FLAG: &[u8] = b"main_quest_lv7.refine_chance";
/// The event flag that stops refining for five minutes after it is set.
const UPDATE_REFINE_TIME_FLAG: &[u8] = b"update_refine_time";
/// `60 * 5`: the seconds `update_refine_time` holds refining.
const UPDATE_REFINE_SECONDS: i64 = 60 * 5;
/// The refine set at which a money-only request is a hack (`G/input_main.cpp:3368`).
const DEVIL_TOWER_SET_LIMIT: u16 = 500;
/// The level `main_quest_lv7`'s free refine covers (`CheckItemUseLevel(20)`).
const REFINE_CHANCE_LEVEL: i32 = 20;

/// `CItem::CheckItemUseLevel(level)` (`G/item.cpp:2634-2645`): the first level limit is at
/// most `level`, or there is none.
fn usable_at_level(proto: &ItemProto, level: i32) -> bool {
    proto
        .limits
        .iter()
        .find(|limit| limit.kind == LIMIT_LEVEL)
        .is_none_or(|limit| limit.value <= level)
}

impl GameState {
    /// Install the refine recipes (`CRefineManager::Initialize`).
    #[must_use]
    pub fn with_refine_recipes(mut self, recipes: Arc<RefineRecipes>) -> Self {
        self.refine_recipes = recipes;
        self
    }

    /// Whether a smith's window is open for the character under `vid`.
    pub(super) fn under_refine(&self, vid: Vid) -> bool {
        self.refining.contains(&vid)
    }

    /// A quest flag of the character `pid`, 0 when it has none.
    fn refine_flag(&self, pid: u32, name: &[u8]) -> i32 {
        self.quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, name))
            .unwrap_or(0)
    }

    /// A line for the refining character.
    pub(super) fn refine_line(&self, actor: Mover, chat_type: u8, format: &[u8]) -> Vec<u8> {
        crate::chat_line::chat_packet(actor.recipient(&self.locale), chat_type, format, &[])
    }

    /// `ReceiveItem`'s smith arms (`G/char_item.cpp:9452-9481`) for a gift `CanReceiveItem`
    /// took: the records the giver under `vid` is sent. A devil-tower smith (`money_only`)
    /// refuses a refine set of 0 or from 500 with `[LS;1002]`; the window then opens for the
    /// gift's cell.
    ///
    /// Only an inventory cell opens the window: legacy reads
    /// `GetInventoryItem(item->GetCell())`, which for a worn item names the inventory item in
    /// the same numbered cell.
    pub(super) fn smith_gift(
        &mut self,
        vid: Vid,
        at: ItemPos,
        money_only: bool,
        refine_set: u16,
        actor: Mover,
    ) -> Vec<Vec<u8>> {
        if money_only && (refine_set == 0 || refine_set >= DEVIL_TOWER_SET_LIMIT) {
            return vec![self.refine_line(actor, CHAT_TYPE_INFO, NOT_REFINABLE_NOTICE)];
        }
        if at.window_type != EWindows::Inventory as u8 {
            return Vec::new();
        }
        let refine_type = if money_only {
            REFINE_TYPE_MONEY_ONLY
        } else {
            REFINE_TYPE_NORMAL
        };
        self.refine_information(vid, at.cell, refine_type, None, actor)
    }

    /// `CHARACTER::RefineInformation(cell, type, additional)` for the character under `vid`:
    /// the records it is sent, the window or the line that refuses it. A window puts the
    /// character under refine and remembers `additional`, the cell of the scroll a
    /// `CG_ITEM_USE_TO_ITEM` used (`SetRefineMode`; `None` is legacy's -1).
    pub(super) fn refine_information(
        &mut self,
        vid: Vid,
        cell: u16,
        refine_type: u8,
        additional: Option<u16>,
        actor: Mover,
    ) -> Vec<Vec<u8>> {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return Vec::new();
        };
        let pid = character.player_id();
        // `bCell >= INVENTORY_MAX_NUM` outside the custom banks (`:1866-1868`); a custom cell
        // is refused too (see the module documentation).
        let Ok(pos) = u8::try_from(cell) else {
            return Vec::new();
        };
        if cell >= INVENTORY_MAX_NUM {
            return Vec::new();
        }
        let at = ItemPos::new(EWindows::Inventory as u8, cell);
        let Some(item) = character.items().item_at(at) else {
            return Vec::new();
        };
        let src_vnum = item.vnum;
        if refine_type == REFINE_TYPE_MONEY_ONLY && self.refine_flag(pid, DEVIL_TOWER_FLAG) == 0 {
            return vec![self.refine_line(actor, CHAT_TYPE_INFO, DEVIL_TOWER_REFINE_NOTICE)];
        }
        let Some(proto) = self.protos.get(src_vnum) else {
            return Vec::new();
        };
        let result_vnum = proto.refined_vnum;
        if result_vnum == 0 {
            tracing::warn!("RefineInformation p.result_vnum == 0");
            return vec![self.refine_line(actor, CHAT_TYPE_INFO, NOT_REFINABLE_NOTICE)];
        }
        if proto.item_type == ITEM_USE && proto.sub_type == USE_TUNING {
            // A scroll refined: the smith's type refuses it, and another needs a scroll other
            // than this one in the additional cell (`:1904-1920`).
            if refine_type == REFINE_TYPE_NORMAL {
                return vec![self.refine_line(actor, CHAT_TYPE_INFO, SCROLL_AT_SMITH_NOTICE)];
            }
            let scroll = additional.and_then(|cell| {
                character
                    .items()
                    .item_at(ItemPos::new(EWindows::Inventory as u8, cell))
            });
            if scroll.is_none_or(|scroll| scroll.vnum == src_vnum) {
                return vec![
                    self.refine_line(actor, CHAT_TYPE_INFO, b"[LS;1106]"),
                    self.refine_line(actor, CHAT_TYPE_INFO, b"[LS;1096]"),
                ];
            }
        }
        let Some(recipe) = self.refine_recipes.get(u32::from(proto.refine_set)) else {
            tracing::warn!("RefineInformation NOT GET REFINE SET {}", proto.refine_set);
            return vec![self.refine_line(actor, CHAT_TYPE_INFO, NOT_REFINABLE_NOTICE)];
        };
        let cost = if self.refine_flag(pid, REFINE_CHANCE_FLAG) > 0 {
            if !usable_at_level(proto, REFINE_CHANCE_LEVEL) || proto.item_type != ITEM_WEAPON {
                return vec![self.refine_line(actor, CHAT_TYPE_INFO, REFINE_CHANCE_NOTICE)];
            }
            0
        } else {
            recipe.cost
        };
        let mut window = GcRefineInformation {
            refine_type,
            pos,
            src_vnum,
            result_vnum,
            material_count: 0,
            cost,
            prob: recipe.prob,
            materials: [RefineMaterial::default(); REFINE_MATERIAL_SLOTS],
        };
        if refine_type != REFINE_TYPE_MONEY_ONLY {
            window.material_count = recipe.material_count;
            for (slot, material) in window.materials.iter_mut().zip(recipe.materials) {
                *slot = RefineMaterial {
                    vnum: material.vnum,
                    count: material.count,
                };
            }
        }
        self.refining.insert(vid);
        match additional {
            Some(scroll) => self.refine_scroll.insert(vid, scroll),
            None => self.refine_scroll.remove(&vid),
        };
        vec![window.encode()]
    }

    /// Run one `CG_REFINE` for the character online under `vid` (`CInputMain::Refine`,
    /// `G/input_main.cpp:3316-3391`). Every request ends the character's refine mode.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn refine(
        &mut self,
        vid: Vid,
        pos: u8,
        refine_type: u8,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let usable = usable_inventory_cells(character.inven_point());
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
        if self.trading.contains_key(&vid)
            || self.safebox_open(vid)
            || self.browsing.contains_key(&vid)
        {
            moved
                .records
                .push(self.refine_line(actor, CHAT_TYPE_INFO, REFINE_BUSY_NOTICE));
            self.refining.remove(&vid);
            return Ok(moved);
        }
        let at = ItemPos::new(EWindows::Inventory as u8, u16::from(pos));
        // The cancel, a cell past the unlocked inventory (a `BYTE` never reaches the custom
        // banks), and an empty cell end the refine mode alone.
        if refine_type == REFINE_TYPE_CANCEL
            || u16::from(pos) >= usable
            || character.items().item_at(at).is_none()
        {
            self.refining.remove(&vid);
            return Ok(moved);
        }
        self.set_refine_time(vid);
        match refine_type {
            REFINE_TYPE_NORMAL => self.do_refine(vid, at, false, actor, &mut moved),
            REFINE_TYPE_MONEY_ONLY => {
                let set = character_refine_set(self, vid, at);
                if set >= DEVIL_TOWER_SET_LIMIT {
                    tracing::warn!(pid, "DEVIL_TOWER_REFINE_HACK");
                } else if self.refine_flag(pid, DEVIL_TOWER_FLAG) != 0 {
                    self.do_refine(vid, at, true, actor, &mut moved);
                    if let Some(quests) = self.quests.as_mut() {
                        quests
                            .manager_mut()
                            .set_quest_flag(pid, DEVIL_TOWER_FLAG, 0);
                    }
                } else {
                    moved.records.push(self.refine_line(
                        actor,
                        CHAT_TYPE_INFO,
                        DEVIL_TOWER_REFINE_NOTICE,
                    ));
                }
            }
            REFINE_TYPE_SCROLL | REFINE_TYPE_HYUNIRON | REFINE_TYPE_MUSIN | REFINE_TYPE_BDRAGON => {
                self.do_refine_with_scroll(vid, at, actor, &mut moved);
            }
            _ => {}
        }
        self.refining.remove(&vid);
        Ok(moved)
    }

    /// `CHARACTER::DoRefine(item, money_only)` (`G/char_item.cpp:1421-1585`) on the item at
    /// `at`, adding what it does to `moved`.
    fn do_refine(
        &mut self,
        vid: Vid,
        at: ItemPos,
        money_only: bool,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        // `G/char_item.cpp:1425-1431`, first; the popup chats.
        if self.is_secured(vid) {
            moved.records.push(self.protected_popup(actor));
            return;
        }
        match self.refine_plan(vid, at, money_only, moved.owner_id) {
            Ok(plan) => self.apply_refine(vid, at, &plan, actor, moved),
            Err(Some(notice)) => {
                moved
                    .records
                    .push(self.refine_line(actor, CHAT_TYPE_INFO, notice));
            }
            Err(None) => {}
        }
    }

    /// `DoRefine`'s gates, in legacy's order: the refine it may run, or the line that
    /// refuses it (`None` for a silent refusal). The free refine of `main_quest_lv7` is
    /// spent when it passes its own gate, as legacy decrements it before the later ones.
    fn refine_plan(
        &mut self,
        vid: Vid,
        at: ItemPos,
        money_only: bool,
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
        let mut cost = recipe.cost;
        let chance = self.refine_flag(pid, REFINE_CHANCE_FLAG);
        if chance > 0 {
            if !usable_at_level(&proto, REFINE_CHANCE_LEVEL) || proto.item_type != ITEM_WEAPON {
                return Err(Some(REFINE_CHANCE_NOTICE));
            }
            cost = 0;
            if let Some(quests) = self.quests.as_mut() {
                quests
                    .manager_mut()
                    .set_quest_flag(pid, REFINE_CHANCE_FLAG, chance - 1);
            }
        }
        let result_vnum = proto.refined_vnum;
        if result_vnum == 0 {
            return Err(Some(NO_RESULT_NOTICE));
        }
        if proto.item_type == ITEM_USE && proto.sub_type == USE_TUNING {
            return Err(None);
        }
        let Some(made) = self.protos.get(result_vnum).cloned() else {
            tracing::error!("DoRefine NOT GET ITEM PROTO {result_vnum}");
            return Err(Some(NOT_REFINABLE_NOTICE));
        };
        // `GetGold() < static_cast<unsigned long long>(cost)`: a negative cost converts to a
        // huge one, which no purse holds.
        let fee = u64::try_from(cost).map_err(|_| Some(REFINE_GOLD_NOTICE))?;
        if gold < fee {
            return Err(Some(REFINE_GOLD_NOTICE));
        }
        let materials: Vec<(u32, i32)> = if !money_only && chance == 0 {
            recipe
                .materials
                .iter()
                .take(usize::from(recipe.material_count))
                .map(|material| (material.vnum, material.count))
                .collect()
        } else {
            Vec::new()
        };
        if materials
            .iter()
            .any(|&(vnum, count)| self.count_specify_item(vid, vnum) < count)
        {
            return Err(Some(REFINE_MATERIAL_NOTICE));
        }
        if !self.refined_fits(vid, at, &made) {
            return Err(None);
        }
        Ok(RefinePlan {
            old,
            proto,
            made,
            prob: recipe.prob,
            fee,
            gold,
            materials,
            money_only,
            fail: FailOutcome::Destroy,
            scroll: None,
        })
    }

    /// `CanHandleItem(true)` with the windows this build has, and the five minutes the
    /// `update_refine_time` event flag holds every refine (`G/char_item.cpp:1440-1447`,
    /// `:1613-1621`).
    pub(super) fn may_refine(&self, vid: Vid, pid: u32) -> bool {
        if self.sash_windows.contains_key(&vid)
            || self.changelook_windows.contains(&vid)
            || self.aura_windows.contains_key(&vid)
        {
            return false;
        }
        let held = self.quests.as_ref().map_or(0, |quests| {
            quests.manager().event_flag(UPDATE_REFINE_TIME_FLAG)
        });
        if held != 0 && i64::from(Self::global_time()) < i64::from(held) + UPDATE_REFINE_SECONDS {
            tracing::info!("can't refine {pid}");
            return false;
        }
        true
    }

    /// Not legacy: whether an item of `made` fits the cell `at` of the character under
    /// `vid` once the refined item leaves it, checked before anything changes.
    pub(super) fn refined_fits(&self, vid: Vid, at: ItemPos, made: &ItemProto) -> bool {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let items = character.items();
        let [size, ..] = made.size.to_le_bytes();
        items.is_empty_item_grid(
            at,
            size,
            Some(at.cell),
            usable_inventory_cells(character.inven_point()),
            belt_grade(items, &self.protos),
        )
    }

    /// The rest of `DoRefine` and `DoRefineWithScroll` once their gates pass: the materials
    /// are taken, the draw is made, a scroll is spent, the command line is sent, the item is
    /// swapped, destroyed or kept, and the fee is paid.
    pub(super) fn apply_refine(
        &mut self,
        vid: Vid,
        at: ItemPos,
        plan: &RefinePlan,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        let pid = moved.owner_id;
        let mut draw = world::character::number(&mut self.dice, 1, 100);
        if plan.money_only {
            draw -= 10;
        }
        let success = i64::from(draw) <= i64::from(plan.prob);
        let protos = Arc::clone(&self.protos);
        // `None` keeps the item, `Some(None)` destroys it, `Some(Some(_))` swaps it.
        let outcome = if success {
            Some(Some((plan.proto.refined_vnum, Some(&plan.made))))
        } else {
            match plan.fail {
                FailOutcome::Destroy => Some(None),
                FailOutcome::Become(vnum) => Some(Some((vnum, protos.get(vnum)))),
                FailOutcome::Keep => None,
            }
        };
        let fresh = match outcome {
            Some(Some((vnum, made))) => {
                let Some(made) = made else {
                    tracing::error!("cannot create item {vnum}");
                    return;
                };
                let Some(id) = self.item_ids.as_mut().and_then(|ids| ids.allocate().ok()) else {
                    tracing::error!("cannot create item {vnum}");
                    return;
                };
                Some(Some(world::character::refined_item(
                    &plan.old,
                    &plan.proto,
                    (id, vnum),
                    made,
                    i64::from(Self::global_time()),
                )))
            }
            Some(None) => Some(None),
            None => None,
        };
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let usable = usable_inventory_cells(character.inven_point());
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let mut spent = MoveDone {
            kind: MoveKind::Refined,
            records: Vec::new(),
            changes: Vec::new(),
        };
        for &(vnum, count) in &plan.materials {
            let removed = world::character::remove_specify_item(
                items,
                slots,
                &protos,
                usable,
                (vnum, u32::try_from(count).unwrap_or(0)),
            );
            spent.records.extend(removed.records);
            spent.changes.extend(removed.changes);
        }
        if let Some((scroll, scroll_proto)) = &plan.scroll {
            if let Ok(mut used) = world::character::spend_one(items, scroll, scroll_proto) {
                world::character::sync_quickslots(&mut used, slots, items);
                spent.records.extend(used.records);
                spent.changes.extend(used.changes);
            }
        }
        let mut swapped = match &fresh {
            Some(made) => world::character::swap_refined(items, at, made.as_ref()),
            None => MoveDone {
                kind: MoveKind::Refined,
                records: Vec::new(),
                changes: Vec::new(),
            },
        };
        world::character::sync_quickslots(&mut swapped, slots, items);
        let left = plan.gold - plan.fee;
        character.set_gold(left);
        swapped.records.push(MoveRecord::Gold {
            amount: 0,
            value: left,
        });
        let quickslots = character.quickslots().clone();
        let points = character.points().cloned();
        let spent = MovedItems::new(pid, vid.raw(), spent, actor, &self.protos, &self.locale);
        let swapped = MovedItems::new(pid, vid.raw(), swapped, actor, &self.protos, &self.locale);
        moved.records.extend(spent.records);
        let command = if success {
            REFINE_SUCCEEDED
        } else {
            REFINE_FAILED
        };
        moved
            .records
            .push(self.refine_line(actor, CHAT_TYPE_COMMAND, command));
        moved.records.extend(swapped.records);
        moved.changes.extend(spent.changes);
        moved.changes.extend(swapped.changes);
        moved.gold = Some(gold_delta(plan.gold, left));
        moved.points = points;
        moved.quickslots = Some(quickslots);
    }
}

/// A refine whose gates passed.
pub(super) struct RefinePlan {
    /// The item refined.
    pub(super) old: world::item::Item,
    /// Its proto.
    pub(super) proto: ItemProto,
    /// The proto of the item a success makes.
    pub(super) made: ItemProto,
    /// The success percent.
    pub(super) prob: i32,
    /// The fee.
    pub(super) fee: u64,
    /// The gold held before the fee.
    pub(super) gold: u64,
    /// The materials taken, empty for a money-only or free refine.
    pub(super) materials: Vec<(u32, i32)>,
    /// A devil-tower refine, whose draw is 10 lower.
    pub(super) money_only: bool,
    /// What a failed draw does to the item.
    pub(super) fail: FailOutcome,
    /// The scroll a scroll refine spends one of, with its proto.
    pub(super) scroll: Option<(world::item::Item, ItemProto)>,
}

/// What a failed refine does to the item.
pub(super) enum FailOutcome {
    /// `DoRefine` destroys it.
    Destroy,
    /// `DoRefineWithScroll` swaps it for the item it was refined from, of this vnum.
    Become(u32),
    /// `DoRefineWithScroll` keeps it: there is no item it was refined from, or the scroll
    /// breaks the item on a failure (`HYUNIRON_CHN`) and legacy's last arm still keeps it.
    Keep,
}

/// The refine set of the item at `at` of the character under `vid`, 0 when there is none.
fn character_refine_set(world: &GameState, vid: Vid, at: ItemPos) -> u16 {
    world
        .characters
        .find_by_vid(vid)
        .ok()
        .and_then(|character| character.items().item_at(at))
        .and_then(|item| world.protos.get(item.vnum))
        .map_or(0, |proto| proto.refine_set)
}

#[cfg(test)]
mod tests {
    use gamedata::records::RefineTableRecord;
    use world::item::{Item, ItemIdRange};

    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};
    use db::items::RowChange;

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

    /// A world whose sword 10 refines with recipe 1 at `prob`, held by player 1 in cell 0
    /// with a stone in socket 0 and 1000 gold.
    fn a_smithy(prob: i32) -> GameState {
        let clock = TestClock::default();
        let recipe = RefineTableRecord {
            id: 1,
            cost: 600,
            prob,
            ..RefineTableRecord::default()
        };
        let mut world =
            a_world(&clock).with_refine_recipes(Arc::new(RefineRecipes::from_records([recipe])));
        world
            .install_item_ids(ItemIdRange::new(100, 1_000_000, 100).expect("a usable range"))
            .expect("the first install");
        let mut sword = Item::new(7, 10);
        sword.size =
            u8::try_from(world.protos().get(10).expect("sword 10").size).expect("a small size");
        sword.sockets[0] = 28_030;
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let (_inbox, _) = enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, &[(at, sword)]);
        world
            .characters
            .find_by_vid_mut(Vid::new(1))
            .expect("player 1")
            .set_gold(1000);
        world
    }

    fn says(moved: &MovedItems, text: &[u8]) -> bool {
        moved
            .records
            .iter()
            .any(|record| record.windows(text.len()).any(|window| window == text))
    }

    /// A refine at 100% swaps the sword for the next one, keeping its stone, and takes the
    /// fee; it stamps the refine time, which `IsHack` then refuses a warp for.
    #[test]
    fn a_sure_refine_swaps_the_item_and_takes_the_fee() {
        let mut world = a_smithy(100);
        let vid = Vid::new(1);
        assert!(world.is_hack(vid, mover()).is_none(), "nothing refused yet");
        let moved = world
            .refine(vid, 0, REFINE_TYPE_NORMAL, mover())
            .expect("player 1");
        assert!(says(&moved, REFINE_SUCCEEDED));
        assert!(matches!(moved.changes[0], RowChange::Destroyed { id: 7 }));
        let RowChange::Created(row) = &moved.changes[1] else {
            panic!("the +1 sword is made: {:?}", moved.changes);
        };
        assert_eq!((row.vnum, row.pos, row.sockets[0]), (11, 0, 28_030));
        assert_eq!(moved.gold, Some(-600));
        assert_eq!(
            world.characters.find_by_vid(vid).expect("player 1").gold(),
            400
        );
        assert!(!world.under_refine(vid));
        assert!(
            world.is_hack(vid, mover()).is_some(),
            "a refine holds the warp"
        );
    }

    /// A refine at 0% destroys the sword and still takes the fee.
    #[test]
    fn a_failed_refine_destroys_the_item_and_takes_the_fee() {
        let mut world = a_smithy(0);
        let vid = Vid::new(1);
        let moved = world
            .refine(vid, 0, REFINE_TYPE_NORMAL, mover())
            .expect("player 1");
        assert!(says(&moved, REFINE_FAILED));
        assert_eq!(moved.changes.len(), 1, "{:?}", moved.changes);
        assert!(matches!(moved.changes[0], RowChange::Destroyed { id: 7 }));
        assert_eq!(moved.gold, Some(-600));
    }
}
