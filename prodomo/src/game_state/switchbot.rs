//! The switchbot: `CG_SWITCHBOT` (`CInputMain::Switchbot`,
//! `G/input_main.cpp:5411-5455`), which starts and stops the bonus changer the
//! table carrier runs (`CSwitchbotManager`, `G/refactorized_switchbot.cpp:1-759`).
//!
//! A switchbot belongs to a player id: five slots, each holding an item id, an
//! active and a finished flag, and two alternatives of five wanted attributes.
//! START arms a slot with its alternatives and lights the 0.08-second event; STOP
//! douses it. Every tick each active slot checks its item against its configured
//! alternatives — every wanted type met at or above its wanted value — and a match
//! whispers the finish, douses the slot, and stops the event when no slot stays
//! active. A miss spends one switcher (39028, 71084 or 76014, the first the purse
//! holds) and changes the item's attributes, sending the item copy; with no
//! switcher the bot stops and chats the out-of-switchers line.
//!
//! The P2P carry (`P2PSendSwitchbot`, `P2PReceiveSwitchbot`) is retired with the
//! game-to-game protocol (ADR-0001): one process holds every Channel, so the table
//! never leaves memory and the warp only pauses the event. The yang price arm
//! (`SWITCHBOT_PRICE_TYPE == 2`) is dead: the compiled constant is 1
//! (`C/length.h:941`).

use std::collections::HashMap;

use common::vid::Vid;
use protocol::cg_switchbot::{
    CgSwitchbotAlternative, SUBHEADER_CG_SWITCHBOT_START, SUBHEADER_CG_SWITCHBOT_STOP,
};
use protocol::gc_item_window::ItemAttribute;
use protocol::gc_whisper::{GcWhisper, WHISPER_TYPE_SYSTEM};

use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// `SWITCHBOT_SLOT_COUNT` (`C/length.h:937`): the five slots.
pub const SWITCHBOT_SLOTS: usize = 5;
/// `SWITCHBOT_ALTERNATIVE_COUNT` (`C/length.h:938`): the two alternatives.
pub const SWITCHBOT_ALTERNATIVES: usize = 2;
/// `c_arSwitchingItems` (`C/length.h:946-951`): the switchers, in spend order.
pub const SWITCHBOT_SWITCHERS: [u32; 3] = [39_028, 71_084, 76_014];
/// `SWITCHBOT_PRICE_AMOUNT` (`C/length.h:943`): one switcher a change.
pub const SWITCHBOT_PRICE: u32 = 1;
/// `c_fSpeed` (`G/refactorized_switchbot.cpp:16`): 0.08 seconds a tick, 2 Pulses.
pub const SWITCHBOT_PERIOD: u64 = 2;

/// One switchbot: the `TSwitchbotTable` and the armed tick.
#[derive(Debug, Clone, Default)]
pub struct Switchbot {
    /// Legacy `items`: the item id per slot, 0 when empty.
    pub items: [u32; SWITCHBOT_SLOTS],
    /// Legacy `active`: the slots the event changes.
    pub active: [bool; SWITCHBOT_SLOTS],
    /// Legacy `finished`: the slots whose wanted attributes landed.
    pub finished: [bool; SWITCHBOT_SLOTS],
    /// Legacy `alternatives`: per slot, per alternative, the five wanted attributes.
    pub alternatives: [[[ItemAttribute; 5]; SWITCHBOT_ALTERNATIVES]; SWITCHBOT_SLOTS],
    /// The Pulse the next tick fires on, or `None` when the event is down.
    pub due: Option<u64>,
    /// `m_isWarping`: set on warp, cleared on enter.
    pub warping: bool,
}

impl GameState {
    /// Run one `CG_SWITCHBOT` for the character online under `vid`
    /// (`CInputMain::Switchbot`, `G/input_main.cpp:5411-5455`).
    ///
    /// The prefix's size is dead and the slot is unchecked here: `ValidPosition`
    /// refuses it inside the start and stop. START needs the 30 body bytes; a short
    /// frame closes the descriptor, as a malformed client frame does. An unknown
    /// subheader runs nothing, as the switch's missing `default:` does.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn switchbot(
        &mut self,
        vid: Vid,
        subheader: u8,
        slot: u8,
        bodies: &[u8],
        mover: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let mut table_out = MovedItems::new(
            pid,
            vid.raw(),
            world::character::MoveDone {
                kind: world::character::MoveKind::Switched,
                records: Vec::new(),
                changes: Vec::new(),
            },
            mover,
            &self.protos,
            &self.locale,
        );
        match subheader {
            SUBHEADER_CG_SWITCHBOT_START => {
                // A short body leaves the defaults: the frame sizer proved the 30
                // bytes on the wire path, but a `GameCommand` may carry fewer.
                let mut alternatives = [[ItemAttribute::new(0, 0); 5]; SWITCHBOT_ALTERNATIVES];
                for (index, alternative) in alternatives.iter_mut().enumerate() {
                    let at = index * 15;
                    if let Some(chunk) = bodies.get(at..at + 15) {
                        if let Ok(parsed) = CgSwitchbotAlternative::decode(chunk) {
                            *alternative = parsed.attributes;
                        }
                    }
                }
                self.switchbot_start(pid, slot, alternatives, &mut table_out);
            }
            SUBHEADER_CG_SWITCHBOT_STOP => {
                self.switchbot_stop(pid, slot, &mut table_out);
            }
            _ => {}
        }
        Ok(table_out)
    }

    /// The switchbots, keyed by player id (`m_map_Switchbots`).
    fn switchbots(&mut self) -> &mut HashMap<u32, Switchbot> {
        &mut self.switchbots
    }

    /// `CSwitchbotManager::Start` (`:525-552`): past the slots, or with no table, or
    /// on an already-running slot, nothing runs (legacy only logs). Else the slot
    /// lights with its alternatives, and the event arms when it is down.
    fn switchbot_start(
        &mut self,
        pid: u32,
        slot: u8,
        alternatives: [[ItemAttribute; 5]; SWITCHBOT_ALTERNATIVES],
        moved: &mut MovedItems,
    ) {
        let slot = usize::from(slot);
        if slot >= SWITCHBOT_SLOTS {
            return;
        }
        let due = self.last_pulse + SWITCHBOT_PERIOD;
        let Some(bot) = self.switchbots().get_mut(&pid) else {
            return;
        };
        if bot.active[slot] {
            return;
        }
        bot.active[slot] = true;
        bot.finished[slot] = false;
        bot.alternatives[slot] = alternatives;
        if bot.due.is_none() {
            bot.due = Some(due);
        }
        self.send_switchbot_update(pid, moved);
    }

    /// `CSwitchbotManager::Stop` (`:554-581`): past the slots, or with no table, or
    /// on a slot that is not running, nothing runs. Else the slot douses, and the
    /// event stops when no slot stays active.
    fn switchbot_stop(&mut self, pid: u32, slot: u8, moved: &mut MovedItems) {
        let slot = usize::from(slot);
        if slot >= SWITCHBOT_SLOTS {
            return;
        }
        let Some(bot) = self.switchbots().get_mut(&pid) else {
            return;
        };
        if !bot.active[slot] {
            return;
        }
        bot.active[slot] = false;
        bot.finished[slot] = false;
        if !bot.active.iter().any(|active| *active) {
            bot.due = None;
            for active in &mut bot.active {
                *active = false;
            }
        }
        self.send_switchbot_update(pid, moved);
    }

    /// `CSwitchbotManager::RegisterItem` (`:494-503`): a table is made for a new
    /// player id, and the slot names the item — unless the bot is warping, and then
    /// nothing runs. The update rides with the move.
    pub(super) fn switchbot_register(&mut self, pid: u32, cell: u16, item_id: u32) {
        let slot = usize::from(cell);
        if slot >= SWITCHBOT_SLOTS {
            return;
        }
        let bot = self.switchbots().entry(pid).or_default();
        if bot.warping {
            return;
        }
        bot.items[slot] = item_id;
    }

    /// `CSwitchbotManager::UnregisterItem` (`:505-519`): the slot forgets the item,
    /// douses, unfinishes, and clears its alternatives — unless the bot is warping.
    pub(super) fn switchbot_unregister(&mut self, pid: u32, cell: u16) {
        let slot = usize::from(cell);
        if slot >= SWITCHBOT_SLOTS {
            return;
        }
        let Some(bot) = self.switchbots().get_mut(&pid) else {
            return;
        };
        if bot.warping {
            return;
        }
        bot.items[slot] = 0;
        bot.active[slot] = false;
        bot.finished[slot] = false;
        bot.alternatives[slot] = [[ItemAttribute::new(0, 0); 5]; SWITCHBOT_ALTERNATIVES];
    }

    /// Whether `from` is a switchbot slot the bot runs: the move source gate
    /// (`G/char_item.cpp:7703-7707`).
    pub(super) fn switchbot_source_active(
        &self,
        vid: Vid,
        from: protocol::item_pos::ItemPos,
    ) -> bool {
        if from.window_type != common::item_slots::EWindows::Switchbot as u8 {
            return false;
        }
        self.characters
            .find_by_vid(vid)
            .is_ok_and(|character| self.switchbot_active(character.player_id(), from.cell))
    }

    /// Follow a move that landed: a switchbot slot left behind unregisters, a slot
    /// landed on registers, and the update answers when a slot was touched
    /// (`CHARACTER::SetItem`, `G/char_item.cpp:535-561`, into
    /// `CSwitchbotManager::RegisterItem`'s update, `:482-503`).
    pub(super) fn switchbot_follow_move(
        &mut self,
        vid: Vid,
        from: protocol::item_pos::ItemPos,
        to: protocol::item_pos::ItemPos,
        moved: &mut MovedItems,
    ) {
        use common::item_slots::EWindows;
        let touched = from.window_type == EWindows::Switchbot as u8
            || to.window_type == EWindows::Switchbot as u8;
        if !touched {
            return;
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        if from.window_type == EWindows::Switchbot as u8 {
            self.switchbot_unregister(pid, from.cell);
        }
        if to.window_type == EWindows::Switchbot as u8 {
            let id = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.items().item_at(to))
                .map(|item| item.id);
            if let Some(id) = id {
                self.switchbot_register(pid, to.cell, id);
            }
        }
        self.send_switchbot_update(pid, moved);
    }

    /// `CSwitchbotManager::SetIsWarping` (`:615-625`): the flag a warp sets and the
    /// enter-game clears, which holds register and unregister still while set.
    pub(super) fn switchbot_warping(&mut self, vid: Vid, warping: bool) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        if let Some(bot) = self.switchbots.get_mut(&pid) {
            bot.warping = warping;
        }
    }

    /// Whether `pid`'s `slot` runs (`CSwitchbotManager::IsActive`, `:583-597`).
    pub(super) fn switchbot_active(&self, pid: u32, slot: u16) -> bool {
        let slot = usize::from(slot);
        slot < SWITCHBOT_SLOTS
            && self
                .switchbots
                .get(&pid)
                .is_some_and(|bot| bot.active[slot])
    }

    /// The enter-game report (`CSwitchbotManager::EnterGame`, `:748-761`): the
    /// attribute info, then the table, sent for every character, with warping off
    /// and the event re-armed behind active slots. Nothing runs when nobody is
    /// online under `vid`.
    pub(super) fn switchbot_report(&mut self, vid: Vid) -> Vec<Vec<u8>> {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return Vec::new();
        };
        let pid = character.player_id();
        let mut entries = Vec::new();
        for set in 0..gamedata::records::ATTRIBUTE_SET_MAX_NUM {
            for record in self.item_attrs.iter() {
                let max = record
                    .max_level_by_set
                    .get(set)
                    .copied()
                    .unwrap_or_default();
                let level = usize::from(max);
                if level == 0 {
                    continue;
                }
                let Some(value) = record.values.get(level - 1).copied() else {
                    continue;
                };
                entries.push(protocol::gc_switchbot::GcSwitchbotAttrEntry {
                    attribute_set: u8::try_from(set).unwrap_or(u8::MAX),
                    apply_num: i32::try_from(record.apply_index).unwrap_or(i32::MAX),
                    max_value: value,
                });
            }
        }
        let mut lines = vec![protocol::gc_switchbot::GcSwitchbotAttrInfo { entries }.encode()];
        let Some(bot) = self.switchbots.get_mut(&pid) else {
            return lines;
        };
        bot.warping = false;
        let table = protocol::gc_switchbot::GcSwitchbotTable {
            player_id: pid,
            active: bot.active,
            finished: bot.finished,
            items: bot.items,
            alternatives: bot.alternatives,
        };
        lines.push(protocol::gc_switchbot::GcSwitchbotUpdate { table }.encode());
        if bot.due.is_none() && bot.active.iter().any(|active| *active) {
            bot.due = Some(self.last_pulse + SWITCHBOT_PERIOD);
        }
        lines
    }

    /// `CSwitchbotManager::SendSwitchbotUpdate` (`:712-742`): the table to the
    /// player. Nothing runs when the table or the character is gone.
    fn send_switchbot_update(&mut self, pid: u32, moved: &mut MovedItems) {
        let Some(bot) = self.switchbots.get(&pid) else {
            return;
        };
        let table = protocol::gc_switchbot::GcSwitchbotTable {
            player_id: pid,
            active: bot.active,
            finished: bot.finished,
            items: bot.items,
            alternatives: bot.alternatives,
        };
        let record = protocol::gc_switchbot::GcSwitchbotUpdate { table }.encode();
        moved.records.push(record);
    }

    /// The switchbot tick (`switchbot_event`, `:151-175`, every 2 Pulses): each due
    /// table runs its active slots in order.
    pub(super) fn run_switchbot_ticks(&mut self, pulse: u64) {
        let due: Vec<u32> = self
            .switchbots
            .iter()
            .filter(|(_, bot)| bot.due.is_some_and(|due| pulse >= due))
            .map(|(pid, _)| *pid)
            .collect();
        for pid in due {
            self.switchbot_tick(pid, pulse);
        }
    }

    /// `CSwitchbot::SwitchItems` (`:242-388`) for one table: each active slot in
    /// order checks its item, and a match whispers the finish and douses the slot,
    /// stopping the event when no slot stays active. A miss spends a switcher and
    /// changes the attributes with the item copy riding; with no switcher the bot
    /// stops and chats the out-of-switchers line. A gone item is skipped; an item
    /// with no owner stops the tick (`return`, `:258-262`).
    fn switchbot_tick(&mut self, pid: u32, pulse: u64) {
        for slot in 0..SWITCHBOT_SLOTS {
            let active = self
                .switchbots
                .get(&pid)
                .is_some_and(|bot| bot.active[slot]);
            if !active {
                continue;
            }
            if let Some(bot) = self.switchbots.get_mut(&pid) {
                bot.finished[slot] = false;
            }
            let item_id = self.switchbots.get(&pid).map_or(0, |bot| bot.items[slot]);
            let Some(vid) = self.vid_of_player(pid) else {
                continue;
            };
            let Ok(character) = self.characters.find_by_vid(vid) else {
                continue;
            };
            let Some(item) = character.items().item(item_id) else {
                continue;
            };
            let vnum = item.vnum;
            if self.switchbot_matches(pid, slot, item_id) {
                self.switchbot_finish(pid, vid, slot, vnum);
                let stays = self
                    .switchbots
                    .get(&pid)
                    .is_some_and(|bot| bot.active.iter().any(|active| *active));
                if stays {
                    if let Some(mut moved) = self.empty_switched(pid, vid) {
                        self.send_switchbot_update(pid, &mut moved);
                        self.flush_switched(vid, moved);
                    }
                } else {
                    if let Some(bot) = self.switchbots.get_mut(&pid) {
                        bot.due = None;
                        for active in &mut bot.active {
                            *active = false;
                        }
                    }
                    if let Some(mut moved) = self.empty_switched(pid, vid) {
                        self.send_switchbot_update(pid, &mut moved);
                        self.flush_switched(vid, moved);
                    }
                }
            } else if !self.switchbot_funded(vid) {
                if let Some(bot) = self.switchbots.get_mut(&pid) {
                    bot.due = None;
                    for active in &mut bot.active {
                        *active = false;
                    }
                }
                if let Some(mut moved) = self.empty_switched(pid, vid) {
                    self.send_switchbot_update(pid, &mut moved);
                    self.flush_switched(vid, moved);
                }
                if let Some(mover) = self.own_mover(vid) {
                    self.tell(
                        vid,
                        mover,
                        protocol::gc_chat::CHAT_TYPE_INFO,
                        b"@@(r_switchbot.cpp)tradus:Switchbot stopped. Out of switchers.",
                        &[] as &[Arg<'_>],
                    );
                }
                return;
            } else {
                self.switchbot_change(pid, vid, slot, item_id);
            }
        }
        if let Some(bot) = self.switchbots.get_mut(&pid) {
            if bot.due.is_some() {
                bot.due = Some(pulse + SWITCHBOT_PERIOD);
            }
        }
    }

    /// `CSwitchbot::CheckItem` (`:390-433`): whether any configured alternative has
    /// every wanted type met at or above its wanted value. An unconfigured
    /// alternative is skipped, and with none configured the item matches.
    fn switchbot_matches(&self, pid: u32, slot: usize, item_id: u32) -> bool {
        let Some(bot) = self.switchbots.get(&pid) else {
            return false;
        };
        let Some(vid) = self.vid_of_player(pid) else {
            return false;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let Some(item) = character.items().item(item_id) else {
            return false;
        };
        let mut checked = false;
        for alternative in &bot.alternatives[slot] {
            if !alternative
                .iter()
                .any(|wanted| wanted.b_type != 0 && wanted.s_value != 0)
            {
                continue;
            }
            checked = true;
            let mut configured = 0;
            let mut correct = 0;
            for wanted in alternative {
                if wanted.b_type == 0 || wanted.s_value == 0 {
                    continue;
                }
                configured += 1;
                if item.attributes[..5].iter().any(|current| {
                    current.b_type == wanted.b_type && current.s_value >= wanted.s_value
                }) {
                    correct += 1;
                }
            }
            if configured == correct {
                return true;
            }
        }
        !checked
    }

    /// The finish: the whisper, the doused slot, the finished flag.
    fn switchbot_finish(&mut self, pid: u32, vid: Vid, slot: usize, vnum: u32) {
        let name = self.item_get_name(vnum);
        let mut text = b"@@(r_switchbot.cpp)tradus:Bonuschange of ".to_vec();
        text.extend_from_slice(&name);
        text.extend_from_slice(format!(" (Slot: {}) successfully finished.", slot + 1).as_bytes());
        let record = GcWhisper {
            whisper_type: WHISPER_TYPE_SYSTEM,
            name_from: crate::loading_phase::name_bytes_field(b"[Switchbot]"),
            can_format: true,
            text,
        };
        if let Ok(bytes) = record.encode() {
            let _sent = self.write_to_client(vid, bytes);
        }
        if let Some(bot) = self.switchbots.get_mut(&pid) {
            bot.active[slot] = false;
            bot.finished[slot] = true;
        }
    }

    /// Whether the purse holds a switcher: the first of 39028, 71084, 76014 the
    /// counts reach (`:304-335`). The level-40 limit on 71151/76023 is dead: neither
    /// is in the spend list.
    fn switchbot_funded(&self, vid: Vid) -> bool {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let usable = common::item_slots::usable_inventory_cells(character.inven_point());
        SWITCHBOT_SWITCHERS.iter().any(|vnum| {
            world::character::count_specify_item(character.items(), usable, *vnum)
                >= i32::try_from(SWITCHBOT_PRICE).unwrap_or(i32::MAX)
        })
    }

    /// The miss: spend the first held switcher and change the attributes, with the
    /// item copy riding and the rows saved (`refactorized_switchbot.cpp:355-377`).
    ///
    /// Legacy calls `SendItemUpdate` after `ChangeAttribute` unconditionally, so the
    /// copy rides even when the item had no attribute to change (`had == 0`,
    /// `G/item_attribute.cpp:235-236`), and the spend's own records ride with it.
    /// Returning early on an unchanged roll would spend in the world while the client
    /// and the store never learn of it, so everything below the spend flushes.
    fn switchbot_change(&mut self, pid: u32, vid: Vid, slot: usize, item_id: u32) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let usable = common::item_slots::usable_inventory_cells(character.inven_point());
        let spent = SWITCHBOT_SWITCHERS.iter().find(|vnum| {
            world::character::count_specify_item(character.items(), usable, **vnum)
                >= i32::try_from(SWITCHBOT_PRICE).unwrap_or(i32::MAX)
        });
        let Some(spent) = spent.copied() else {
            return;
        };
        let Some(mut moved) = self.empty_switched(pid, vid) else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let mut done = world::character::MoveDone {
            kind: world::character::MoveKind::Switched,
            records: Vec::new(),
            changes: Vec::new(),
        };
        let spent_done = world::character::remove_specify_item(
            items,
            slots,
            &self.protos,
            usable,
            (spent, SWITCHBOT_PRICE),
        );
        done.records.extend(spent_done.records);
        done.changes.extend(spent_done.changes);
        if let (Some(item), Some(proto)) = (
            items.item(item_id).cloned(),
            items
                .item(item_id)
                .and_then(|item| self.protos.get(item.vnum).cloned()),
        ) {
            let before = item.attributes;
            let mut after = before;
            // `CItem::ChangeAttribute` (`G/item_attribute.cpp:228-265`) rolls only
            // when the item had an attribute; the spend above and the copy below
            // ride either way.
            let changed = world::character::change_attribute(
                &mut self.dice,
                &mut self.gauss,
                &proto,
                &mut after,
                &self.item_attrs,
            );
            if changed {
                for (index, attribute) in after.iter().enumerate().take(5) {
                    if before[index] != *attribute {
                        let _ = items.set_attribute(item_id, index, *attribute);
                    }
                }
                let socket3 = item.sockets[3].wrapping_add(1);
                let _ = world::character::set_item_socket(items, item_id, (3, socket3));
                if let Some(item) = items.item(item_id) {
                    done.changes.push(world::character::ItemChange::Attributes {
                        id: item_id,
                        attributes: item.attributes,
                    });
                    done.changes.push(world::character::ItemChange::Sockets {
                        id: item_id,
                        sockets: item.sockets,
                    });
                }
            }
        }
        let Some(actor) = self.own_mover(vid) else {
            return;
        };
        let packed = MovedItems::new(pid, vid.raw(), done, actor, &self.protos, &self.locale);
        moved.records.extend(packed.records);
        moved.changes.extend(packed.changes);
        if let Ok(character) = self.characters.find_by_vid(vid) {
            if let Some(item) = character.items().item(item_id) {
                let copy = protocol::gc_switchbot::GcSwitchbotItem {
                    slot: u8::try_from(slot).unwrap_or(u8::MAX),
                    vnum: (item.vnum & 0xff) as u8,
                    count: u8::try_from(item.count).unwrap_or(u8::MAX),
                    sockets: item.sockets,
                    attributes: item.attributes,
                };
                moved.records.push(copy.encode());
            }
        }
        self.flush_switched(vid, moved);
    }

    /// An empty switched move for `pid` under `vid`; `None` when the mover is gone.
    fn empty_switched(&self, pid: u32, vid: Vid) -> Option<MovedItems> {
        let actor = self.own_mover(vid)?;
        Some(MovedItems::new(
            pid,
            vid.raw(),
            world::character::MoveDone {
                kind: world::character::MoveKind::Switched,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        ))
    }

    /// Flush a tick's move: the records to the client, the rows to its descriptor.
    fn flush_switched(&mut self, vid: Vid, moved: MovedItems) {
        for record in &moved.records {
            let _sent = self.write_to_client(vid, record.clone());
        }
        if moved.changes.is_empty() {
            return;
        }
        let Some((channel, map)) = self.place_of(vid.raw()) else {
            return;
        };
        let order = crate::item_move::StoreOrder::default();
        let _ordered = self.order_the_item_rows(vid, channel, map, moved.changes, order);
    }

    /// The VID of the character holding `pid`, if it is in the world.
    fn vid_of_player(&self, pid: u32) -> Option<Vid> {
        self.characters
            .find_by_pid(pid)
            .ok()
            .map(world::character::Character::vid)
    }
}

#[cfg(test)]
mod tests {
    use common::item_slots::EWindows;
    use common::vid::Vid;
    use protocol::item_pos::ItemPos;
    use world::item::Item;

    use super::super::GameState;
    use super::{SUBHEADER_CG_SWITCHBOT_START, SUBHEADER_CG_SWITCHBOT_STOP};
    use crate::game_state::fixtures::{a_world, enter_seeing_with, TestClock, PLACE};
    use crate::item_move::Mover;

    const BOTTER: Vid = Vid::new(11);
    const PID: u32 = 11;

    fn slot(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Switchbot as u8, cell)
    }

    fn inventory(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    fn held(id: u32, vnum: u32) -> Item {
        let mut item = Item::new(id, vnum);
        item.set_size(1).unwrap();
        item
    }

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

    /// The start's ten wanted attributes: apply 1 at 5 in every cell.
    fn bodies() -> Vec<u8> {
        let mut bodies = Vec::new();
        for _ in 0..10 {
            bodies.push(1);
            bodies.extend(5i16.to_le_bytes());
        }
        bodies
    }

    /// A world where player 11 holds a sword (vnum 19, size 2) in cell 0, with
    /// the character's inbox for the flushed records.
    fn a_botter() -> (GameState, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut sword = Item::new(5, 19);
        sword.set_size(2).unwrap();
        let at = ItemPos::new(EWindows::Inventory as u8, 0);
        let (inbox, _) = enter_seeing_with(&mut world, PLACE, 11, (5, 5), 10_000, &[(at, sword)]);
        (world, inbox)
    }

    /// `Stop`, the move gates, and the warp pause: the slot douses, a potion cannot
    /// land on a slot, an active slot's sword cannot move, and warping stills the
    /// register.
    #[test]
    fn a_stopped_slot_douses_and_the_gates_hold() {
        use world::character::{MoveKind, MoveRefused, MoveRequest};
        let (mut state, _inbox) = a_botter();
        state.switchbot_register(PID, 0, 5);
        state
            .switchbot(BOTTER, SUBHEADER_CG_SWITCHBOT_START, 0, &bodies(), mover())
            .unwrap();
        assert!(state.switchbot_active(PID, 0));
        // An active slot's sword cannot move away.
        let held_back = state.move_item(
            BOTTER,
            MoveRequest {
                from: slot(0),
                to: inventory(1),
                count: 0,
            },
            mover(),
        );
        assert!(
            matches!(
                held_back,
                Err(crate::item_move::MoveItemRefused::Refused(
                    MoveRefused::SwitchbotActive
                ))
            ),
            "an active slot holds its item, {held_back:?}"
        );
        state
            .switchbot(BOTTER, SUBHEADER_CG_SWITCHBOT_STOP, 0, &[], mover())
            .unwrap();
        assert!(!state.switchbot_active(PID, 0));
        // A potion is no switchbot item.
        let mut potion = held(9, 27003);
        potion.count = 1;
        let character = state.characters.find_by_vid_mut(BOTTER).unwrap();
        let (items, _, _) = character.items_points_and_quickslots_mut();
        items.set(inventory(2), &potion).unwrap();
        let alien = state.move_item(
            BOTTER,
            MoveRequest {
                from: inventory(2),
                to: slot(1),
                count: 0,
            },
            mover(),
        );
        assert!(
            matches!(
                alien,
                Err(crate::item_move::MoveItemRefused::Refused(
                    MoveRefused::NotSwitchbotItem
                ))
            ),
            "a potion cannot land on a slot, {alien:?}"
        );
        // Warping stills the register but not the report.
        state.switchbot_warping(BOTTER, true);
        state.switchbot_register(PID, 1, 9);
        assert_eq!(state.switchbots[&PID].items[1], 0);
        let lines = state.switchbot_report(BOTTER);
        assert!(!lines.is_empty(), "the report answers every character");
        assert!(!state.switchbots[&PID].warping, "enter-game clears warping");
        let _ = MoveKind::Switched;
    }

    /// A short START body starts the slot with default alternatives rather than
    /// panicking on the missing bytes.
    #[test]
    fn a_short_start_body_leaves_the_defaults() {
        let (mut state, _inbox) = a_botter();
        state.switchbot_register(PID, 0, 5);
        state
            .switchbot(BOTTER, SUBHEADER_CG_SWITCHBOT_START, 0, &[1, 2, 3], mover())
            .unwrap();
        assert!(state.switchbot_active(PID, 0));
        let empty = [[protocol::gc_item_window::ItemAttribute::new(0, 0); 5]; 2];
        assert_eq!(state.switchbots[&PID].alternatives[0], empty);
    }

    /// A miss on an attribute-less sword still spends the switcher and flushes: the
    /// spend's update and the item copy ride even though `ChangeAttribute` changed
    /// nothing (`refactorized_switchbot.cpp:355-377`, `G/item_attribute.cpp:235-236`).
    #[test]
    fn a_miss_without_attributes_still_spends_and_flushes() {
        let (mut state, mut inbox) = a_botter();
        let mut switchers = held(6, 71084);
        switchers.count = 10;
        let character = state.characters.find_by_vid_mut(BOTTER).unwrap();
        let (items, _, _) = character.items_points_and_quickslots_mut();
        items.set(inventory(1), &switchers).unwrap();
        state.switchbot_register(PID, 0, 5);
        state
            .switchbot(BOTTER, SUBHEADER_CG_SWITCHBOT_START, 0, &bodies(), mover())
            .unwrap();
        // Drain the entry: the enter-game burst and the report ride the inbox.
        while inbox.try_recv().is_ok() {}
        state.switchbot_tick(PID, state.last_pulse + super::SWITCHBOT_PERIOD);
        let character = state.characters.find_by_vid(BOTTER).unwrap();
        let switchers = character.items().item(6).unwrap();
        assert_eq!(switchers.count, 9, "the tick spent one switcher");
        let mut flushed = Vec::new();
        while let Ok(record) = inbox.try_recv() {
            flushed.push(record);
        }
        // The spend's update (a stack that keeps some is updated) and the item
        // copy ride even though the sword had no attribute to change.
        assert!(
            flushed.iter().any(|record| record[0] == 25),
            "the spend's update flushes: {flushed:02x?}"
        );
        assert!(
            flushed.iter().any(
                |record| record.len() == protocol::gc_switchbot::UPDATE_ITEM_LEN
                    && record[0] == protocol::gc_switchbot::HEADER
                    && record[5] == protocol::gc_switchbot::SUB_UPDATE_ITEM
            ),
            "the item copy flushes: {flushed:02x?}"
        );
    }
}
