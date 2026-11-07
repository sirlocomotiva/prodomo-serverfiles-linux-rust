//! The Gaya craft, market and refresh: the `CG_GAYA_SYSTEM` request
//! (`CInputMain::GayaSystemSend`, `G/input_main.cpp:3188-3215`), with no observer gate
//! (`:3881-3883`).
//!
//! The record's header byte is the shadowed one: 241 frames as 67 bytes
//! (`TPacketCGClientVersion2`, first-wins in `CPacketInfoCG`), while the client sends 6
//! (`protocol::cg_gaya_system`). The 6-byte record is decoded; a 67-byte version-shaped
//! frame on this byte never reaches the handler, because the descriptor routes a main-phase
//! 0xf1 of that width to the version step (`prodomo/src/main.rs`), as the login phase does.
//!
//! The craft (`CraftGayaItems`, `G/char_gaya.cpp:275-309`) minerals the stone at
//! `{INVENTORY, pos}`: a missing item sends nothing, a non-stone or one graded past
//! `GRADE_STONE` chats the error line, short of the `GLIMMERSTONE_COUNT` glimmerstones the
//! count line chats with the stone's locale name, and short of `COST_GAYA_YANG` the Yang
//! line chats. Else the roll runs (`number(1, 100)` at or under `PROB_GAYA` wins): a win
//! grants `GIVE_GAYA` Gaya with the grant and success lines, a loss chats the failure line,
//! and either way the window refreshes (`GayaCheck`), the stones and the Yang leave, and the
//! cell loses one stone. `CreateItem` of the stone cannot fail in the Rewrite: the proto is
//! looked up, and a missing one sends nothing before the count line can name it.
//!
//! The market (`MarketGayaItems`, `:311-344`) with no shelf chats the error line, as does a
//! slot past 8 (a negative one is `CheckItemsFull`'s business: the shelf check runs first).
//! Slots 0-2 buy at once (`BuyItemsGayaMarket`, `:115-128`): short of the price the poor
//! line chats, else the Gaya leaves and the item is given. Slots 3-8 unlock with
//! `GAYA_EXPANSION_COUNT` expansion items when locked (`UpdateSlotGayaMarket`, `:153-173`
//! re-sends the markers), and buy once open. A locked slot short of the unlocks chats the
//! count line with the unlock's locale name.
//!
//! The refresh (`RefreshGayaItems`, `:346-367`) with no shelf chats the error line. Short
//! of the `GAYA_REFRESH_COUNT` refreshers the count line chats with the refresher's locale
//! name; else they leave, the shelf re-rolls with the slots kept (`RefreshItemsGayaMarket`,
//! `:130-151`), and the markers are re-sent (`InfoGayaMarker`, `:94-110`).
//!
//! `GetRefineLevel` is the number after the proto name's last `+`
//! (`world::character::refine_level`); the locale-name cross-check only logs. The Yang cost
//! is `PointChange(POINT_GOLD, -cost, false)` with no record of its own. The money log of
//! the buy's give is `sys.log`'s, which owns no sink.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::item_slots::usable_inventory_cells;
use common::vid::Vid;
use gamedata::item_kind::ITEM_METIN;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use protocol::item_pos::ItemPos;
use world::character::{MoveDone, MoveKind, MoveRecord};

use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::item_move::{gold_delta, MoveItemRefused, MovedItems, Mover};

/// `GAYA_SYSTEM_SUB_HEADER_CRAFT`: mineral a stone into Gaya.
pub const GAYA_CRAFT: u8 = 0;
/// `GAYA_SYSTEM_SUB_HEADER_MARKET`: buy from or unlock the market shelf.
pub const GAYA_MARKET: u8 = 1;
/// `GAYA_SYSTEM_SUB_HEADER_REFRESH`: re-roll the market shelf.
pub const GAYA_REFRESH: u8 = 2;

/// The last market shelf slot (`G/char_gaya.cpp:146,318`): nine slots, 0-8.
const GAYA_LAST_SLOT: i32 = 8;
/// The first market slot that unlocks (`MarketGayaItems`, `:326`).
const GAYA_UNLOCK_FIRST: i32 = 3;

/// The poor line (`txt_gaya[0]`).
const GAYA_POOR_NOTICE: &[u8] = b"[Gaya] Nu de\xfeii suficient\xe3 Gaya";
/// The error line (`txt_gaya[1]`).
const GAYA_ERROR_NOTICE: &[u8] = b"[Gaya] Eroare..";
/// The short-of-items line (`txt_gaya[2]`): `%d` of `%s` asked.
const GAYA_COUNT_NOTICE: &[u8] = b"[Gaya] %d unit\xe3\xfei %s cerute.";
/// The poor-Yang line (`txt_gaya[3]`).
const GAYA_YANG_NOTICE: &[u8] = b"[Gaya] Nu ai destul Yang.";
/// The grant line (`txt_gaya[4]`): the Gaya won.
const GAYA_GRANT_NOTICE: &[u8] = b"[Gaya] Ai reu\xbait s\xe3  : %d.";
/// The craft-success line (`txt_gaya[5]`).
const GAYA_WON_NOTICE: &[u8] = b"[Gaya] Ascu\xfeirea a reu\xbait.";
/// The craft-failure line (`txt_gaya[6]`).
const GAYA_LOST_NOTICE: &[u8] = b"[Gaya] Ascu\xfeirea a e\xbauat.";

impl GameState {
    /// A Gaya line as an `INFO` chat. The lines are `LC_TEXT` keys that name no locale
    /// row, so the raw key goes out.
    fn gaya_line(&self, actor: Mover, key: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
        chat_packet(actor.recipient(&self.locale), CHAT_TYPE_INFO, key, args)
    }

    /// Run one `CG_GAYA_SYSTEM` for the character online under `vid`
    /// (`CInputMain::GayaSystemSend`, `G/input_main.cpp:3188-3215`), answered as a move
    /// is. An unknown sub-header is absorbed, as legacy's `default`-less `switch` does.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when no character is online under `vid`.
    pub fn gaya_system(
        &mut self,
        vid: Vid,
        sub_header: u8,
        pos: i32,
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
                kind: MoveKind::Given,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        match sub_header {
            GAYA_CRAFT => self.craft_gaya(vid, pos, actor, &mut moved),
            GAYA_MARKET => self.market_gaya(vid, pos, actor, &mut moved),
            GAYA_REFRESH => self.refresh_gaya(vid, actor, &mut moved),
            _ => {}
        }
        Ok(moved)
    }

    /// `CraftGayaItems` (`G/char_gaya.cpp:275-309`): the guard chain, the roll, and the
    /// spend of a craft of the stone at `{INVENTORY, pos}`.
    fn craft_gaya(&mut self, vid: Vid, pos: i32, actor: Mover, moved: &mut MovedItems) {
        let values = self.gaya_table.values;
        let Ok(cell) = u16::try_from(pos) else {
            return;
        };
        let judged = self.judge_gaya_craft(vid, cell, values, actor);
        match judged {
            CraftJudged::Silent | CraftJudged::NoProto => {}
            CraftJudged::Refused(line) => {
                moved.records.push(line);
            }
            CraftJudged::Runs(spend) => {
                let roll = world::character::number(&mut self.dice, 1, 100);
                self.finish_gaya_craft(vid, spend, roll, values, actor, moved);
            }
        }
    }

    /// `MarketGayaItems` (`G/char_gaya.cpp:311-344`): the shelf and slot guards, then the
    /// unlock or the buy.
    fn market_gaya(&mut self, vid: Vid, pos: i32, actor: Mover, moved: &mut MovedItems) {
        if !self.gaya_markets.contains_key(&vid) {
            moved
                .records
                .push(self.gaya_line(actor, GAYA_ERROR_NOTICE, &[]));
            return;
        }
        if pos > GAYA_LAST_SLOT {
            moved
                .records
                .push(self.gaya_line(actor, GAYA_ERROR_NOTICE, &[]));
            return;
        }
        if pos >= GAYA_UNLOCK_FIRST {
            self.unlock_gaya_slot(vid, pos, actor, moved);
        } else {
            self.buy_gaya_slot(vid, pos, actor, moved);
        }
    }

    /// The locked slot's unlock (`MarketGayaItems`, `:326-340`): the expansion items
    /// leave and the markers re-send; an open slot buys instead, and a locked slot
    /// short of the unlocks chats the count line.
    fn unlock_gaya_slot(&mut self, vid: Vid, pos: i32, actor: Mover, moved: &mut MovedItems) {
        let values = self.gaya_table.values;
        let slot = usize::try_from(pos - GAYA_UNLOCK_FIRST).unwrap_or(usize::MAX);
        let open = self
            .gaya_markets
            .get(&vid)
            .and_then(|market| market.slots.get(slot))
            .is_some_and(|open| *open != 0);
        if open {
            self.buy_gaya_slot(vid, pos, actor, moved);
            return;
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let usable = usable_inventory_cells(character.inven_point());
        let held =
            world::character::count_specify_item(character.items(), usable, values.expansion);
        let need = i32::try_from(values.expansion_count).unwrap_or(i32::MAX);
        if held < need {
            let Some(unlock) = self.protos.get(values.expansion).cloned() else {
                return;
            };
            moved.records.push(self.gaya_line(
                actor,
                GAYA_COUNT_NOTICE,
                &[
                    Arg::Int(values.expansion_count.into()),
                    Arg::Text(&unlock.locale_name),
                ],
            ));
            return;
        }
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let mut done = MoveDone {
            kind: MoveKind::Given,
            records: Vec::new(),
            changes: Vec::new(),
        };
        let protos = std::sync::Arc::clone(&self.protos);
        let spent = world::character::remove_specify_item(
            items,
            slots,
            &protos,
            usable,
            (values.expansion, values.expansion_count),
        );
        done.records.extend(spent.records);
        done.changes.extend(spent.changes);
        world::character::sync_quickslots(&mut done, slots, items);
        let quickslots = slots.clone();
        if let Some(market) = self.gaya_markets.get_mut(&vid) {
            market.slots[slot] = 1;
        }
        let done = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            done,
            actor,
            &self.protos,
            &self.locale,
        );
        moved.records.extend(done.records);
        moved.changes.extend(done.changes);
        moved.quickslots = Some(quickslots);
        self.send_gaya_market(vid, actor);
    }

    /// `RefreshGayaItems` (`G/char_gaya.cpp:346-367`): short of the refreshers the count
    /// line chats; else they leave, the shelf re-rolls with the slots kept, and the
    /// markers re-send.
    fn refresh_gaya(&mut self, vid: Vid, actor: Mover, moved: &mut MovedItems) {
        if !self.gaya_markets.contains_key(&vid) {
            moved
                .records
                .push(self.gaya_line(actor, GAYA_ERROR_NOTICE, &[]));
            return;
        }
        let values = self.gaya_table.values;
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let usable = usable_inventory_cells(character.inven_point());
        let held = world::character::count_specify_item(character.items(), usable, values.refresh);
        let need = i32::try_from(values.refresh_count).unwrap_or(i32::MAX);
        if held < need {
            let Some(refresher) = self.protos.get(values.refresh).cloned() else {
                return;
            };
            moved.records.push(self.gaya_line(
                actor,
                GAYA_COUNT_NOTICE,
                &[
                    Arg::Int(values.refresh_count.into()),
                    Arg::Text(&refresher.locale_name),
                ],
            ));
            return;
        }
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let protos = std::sync::Arc::clone(&self.protos);
        let mut done = MoveDone {
            kind: MoveKind::Given,
            records: Vec::new(),
            changes: Vec::new(),
        };
        let spent = world::character::remove_specify_item(
            items,
            slots,
            &protos,
            usable,
            (values.refresh, values.refresh_count),
        );
        done.records.extend(spent.records);
        done.changes.extend(spent.changes);
        world::character::sync_quickslots(&mut done, slots, items);
        let quickslots = slots.clone();
        let slots = self
            .gaya_markets
            .get(&vid)
            .map_or([0; 6], |market| market.slots);
        self.restock_gaya_market(vid);
        if let Some(market) = self.gaya_markets.get_mut(&vid) {
            market.slots = slots;
        }
        let done = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            done,
            actor,
            &self.protos,
            &self.locale,
        );
        moved.records.extend(done.records);
        moved.changes.extend(done.changes);
        moved.quickslots = Some(quickslots);
        self.send_gaya_market(vid, actor);
    }

    /// The buy (`BuyItemsGayaMarket`, `G/char_gaya.cpp:115-128`): short of the price the
    /// poor line chats; else the Gaya leaves and the shelf's item is given.
    fn buy_gaya_slot(&mut self, vid: Vid, pos: i32, actor: Mover, moved: &mut MovedItems) {
        let slot = usize::try_from(pos).unwrap_or(usize::MAX);
        let Some((vnum, price, count)) = self
            .gaya_markets
            .get(&vid)
            .and_then(|market| market.items.get(slot).copied())
        else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let gaya = character.points().map_or(0, world::character::Points::gaya);
        let price = i32::try_from(price).unwrap_or(i32::MAX);
        if gaya < price {
            moved
                .records
                .push(self.gaya_line(actor, GAYA_POOR_NOTICE, &[]));
            return;
        }
        let spend = price.saturating_neg();
        if let Some(points) = self.pc_points_mut(vid.raw()) {
            if let Ok(records) =
                points.point_change(common::point_slot::POINT_GAYA, spend, false, false)
            {
                let done = MoveDone {
                    kind: MoveKind::Given,
                    records: records.into_iter().map(MoveRecord::Point).collect(),
                    changes: Vec::new(),
                };
                let rendered = MovedItems::new(
                    moved.owner_id,
                    vid.raw(),
                    done,
                    actor,
                    &self.protos,
                    &self.locale,
                );
                moved.records.extend(rendered.records);
            }
        }
        let count = u16::try_from(count).unwrap_or(u16::MAX);
        let Some((outcome, done)) = self.auto_give(vid, (vnum, count), 0, false) else {
            return;
        };
        if let world::character::AutoGiven::Unplaced(item) = outcome {
            self.lay_auto_given(vid, item);
        }
        let points = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points().cloned());
        let quickslots = self
            .characters
            .find_by_vid(vid)
            .ok()
            .map(|character| character.quickslots().clone());
        let done = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            done,
            actor,
            &self.protos,
            &self.locale,
        );
        moved.records.extend(done.records);
        moved.changes.extend(done.changes);
        moved.points = points;
        moved.quickslots = quickslots;
    }

    /// The craft's guards: the spend judged, the refusal line, or silence for a missing
    /// cell or a proto the table names that the protos do not hold.
    fn judge_gaya_craft(
        &self,
        vid: Vid,
        cell: u16,
        values: gamedata::gaya::GayaValues,
        actor: Mover,
    ) -> CraftJudged {
        let at = ItemPos::new(common::item_slots::EWindows::Inventory as u8, cell);
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return CraftJudged::Silent;
        };
        let Some(stone) = character.items().item_at(at).cloned() else {
            return CraftJudged::Silent;
        };
        let Some(proto) = self.protos.get(stone.vnum).cloned() else {
            return CraftJudged::NoProto;
        };
        let grade = i32::try_from(values.grade_stone).unwrap_or(i32::MAX);
        if proto.item_type != ITEM_METIN || world::character::refine_level(&proto.name) > grade {
            return CraftJudged::Refused(self.gaya_line(actor, GAYA_ERROR_NOTICE, &[]));
        }
        let usable = usable_inventory_cells(character.inven_point());
        let held =
            world::character::count_specify_item(character.items(), usable, values.glimmerstone);
        let need = i32::try_from(values.glimmerstone_count).unwrap_or(i32::MAX);
        if held < need {
            let Some(glimmer) = self.protos.get(values.glimmerstone).cloned() else {
                return CraftJudged::NoProto;
            };
            let line = self.gaya_line(
                actor,
                GAYA_COUNT_NOTICE,
                &[
                    Arg::Int(values.glimmerstone_count.into()),
                    Arg::Text(&glimmer.locale_name),
                ],
            );
            return CraftJudged::Refused(line);
        }
        if character.gold() < u64::from(values.cost_yang) {
            return CraftJudged::Refused(self.gaya_line(actor, GAYA_YANG_NOTICE, &[]));
        }
        CraftJudged::Runs(GayaCraft {
            at,
            id: stone.id,
            count: stone.count,
        })
    }
}

/// What the craft's guards judged: silence, a missing proto, a refusal line, or the spend.
enum CraftJudged {
    /// No stone at the cell, or no character: nothing is sent.
    Silent,
    /// A proto the craft names is not loaded: nothing is sent.
    NoProto,
    /// A guard refused: the line.
    Refused(Vec<u8>),
    /// The craft runs: the stone's spend.
    Runs(GayaCraft),
}

/// The stone a running craft minerals: its cell, id and count.
#[derive(Clone, Copy)]
struct GayaCraft {
    /// The stone's cell.
    at: ItemPos,
    /// The stone's id.
    id: world::item::ItemId,
    /// The stone's count.
    count: u16,
}

impl GameState {
    /// The running craft's tail: the win or loss lines, the window refresh, and the
    /// spend — the stones, the Yang, and one stone from the cell.
    fn finish_gaya_craft(
        &mut self,
        vid: Vid,
        spend: GayaCraft,
        roll: i32,
        values: gamedata::gaya::GayaValues,
        actor: Mover,
        moved: &mut MovedItems,
    ) {
        let chance = i32::try_from(values.prob_gaya).unwrap_or(0);
        if roll <= chance {
            let grant = i32::try_from(values.give_gaya).unwrap_or(i32::MAX);
            if let Some(points) = self.pc_points_mut(vid.raw()) {
                if let Ok(records) =
                    points.point_change(common::point_slot::POINT_GAYA, grant, false, false)
                {
                    let done = MoveDone {
                        kind: MoveKind::Given,
                        records: records.into_iter().map(MoveRecord::Point).collect(),
                        changes: Vec::new(),
                    };
                    let rendered = MovedItems::new(
                        moved.owner_id,
                        vid.raw(),
                        done,
                        actor,
                        &self.protos,
                        &self.locale,
                    );
                    moved.records.extend(rendered.records);
                }
            }
            moved.records.push(self.gaya_line(
                actor,
                GAYA_GRANT_NOTICE,
                &[Arg::Int(values.give_gaya.into())],
            ));
            moved
                .records
                .push(self.gaya_line(actor, GAYA_WON_NOTICE, &[]));
        } else {
            moved
                .records
                .push(self.gaya_line(actor, GAYA_LOST_NOTICE, &[]));
        }
        moved.records.push(chat_packet(
            actor.recipient(&self.locale),
            CHAT_TYPE_COMMAND,
            b"GayaCheck",
            &[],
        ));
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let held = character.gold();
        let left = held.saturating_sub(u64::from(values.cost_yang));
        character.set_gold(left);
        let usable = usable_inventory_cells(character.inven_point());
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let mut done = MoveDone {
            kind: MoveKind::Given,
            records: Vec::new(),
            changes: Vec::new(),
        };
        let stones = world::character::remove_specify_item(
            items,
            slots,
            &self.protos,
            usable,
            (values.glimmerstone, values.glimmerstone_count),
        );
        done.records.extend(stones.records);
        done.changes.extend(stones.changes);
        if spend.count == 1 {
            spend_stone(items, slots, &mut done, spend);
        } else if items
            .set_count(spend.id, u32::from(spend.count - 1))
            .is_ok()
        {
            if let Some(stone) = items.item(spend.id) {
                done.records
                    .push(MoveRecord::Item(world::character::ItemRecord::Update(
                        stone.gc_item_update(),
                    )));
            }
            done.changes.push(world::character::ItemChange::Count {
                id: spend.id,
                count: spend.count - 1,
            });
        }
        world::character::sync_quickslots(&mut done, slots, items);
        let quickslots = slots.clone();
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let points = character.points().cloned();
        let done = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            done,
            actor,
            &self.protos,
            &self.locale,
        );
        moved.records.extend(done.records);
        moved.changes.extend(done.changes);
        moved.gold = Some(gold_delta(held, left));
        moved.points = points;
        moved.quickslots = Some(quickslots);
    }
}

/// The last stone of a craft's stack: `SetCount(0)` (`G/item.cpp:303-330`, as
/// `remove_specify_item` runs it): the slots follow or delete, the cell clears, and the
/// row is destroyed.
fn spend_stone(
    items: &mut world::character::CharacterItems,
    slots: &mut world::character::Quickslots,
    done: &mut MoveDone,
    spend: GayaCraft,
) {
    let Some(stone) = items.item(spend.id) else {
        return;
    };
    let sync = world::character::QuickslotSync {
        from: spend.at.cell,
        to: world::character::SyncTo::Delete,
    };
    let deleted = slots.sync(sync, items);
    done.records
        .extend(deleted.into_iter().map(MoveRecord::Quickslot));
    let _ = stone;
    if items.release(spend.id).is_err() {
        return;
    }
    done.records
        .push(MoveRecord::Item(world::character::ItemRecord::Set(
            world::item::gc_item_clear(spend.at),
        )));
    done.changes
        .push(world::character::ItemChange::Destroyed { id: spend.id });
}
