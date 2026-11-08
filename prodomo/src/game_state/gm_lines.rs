//! Thirteen small GM lines: `/item_id_list` (`G/cmd_gm.cpp:4688-4696`), `/get_mob_count`
//! (`:4523-4552`), `/effect` (`:4176-4184`), `/add_socket` (`:3934-3939`), `/set_socket`
//! (`:4698-4713`), `/change_attr` (`:3920-3925`), `/add_attr` (`:3927-3932`),
//! `/change_rare_attr` (`:3942-3947`), `/add_rare_attr` (`:3949-3954`), `/reset_subskill`
//! (`:4315-4334`), `/flush` (`:4425-4446`), `/observer` (`:3226-3229`), `/force_logout`
//! (`G/cmd_general.cpp:3503-3509`) and `/fetch_casket_items` (`:4136-4173`), plus
//! `/break_marriage` (`G/cmd_gm.cpp:4162-4174`) and `/close_shop`
//! (`G/cmd_general.cpp:2004-2012`).
//!
//! [`GameState::run_gm_line`] runs one. The list, the count and the effect store nothing.
//! The socket and attribute lines write an item in place, send its `GC_ITEM_UPDATE` to the
//! character holding it (`CItem::UpdatePacket`, `G/item.cpp:249-276`), and answer the row the
//! caller's descriptor stores; the holder may be another character, whose row it is.
//! `/reset_subskill` answers the named character's zeroed sub-skills, whose rows go;
//! `/flush` has the caller's descriptor write its row now; `/observer` toggles the caller's
//! observer mode; `/force_logout` closes the caller's own descriptor; `/fetch_casket_items`
//! tells the named group's rows and the sort; `/break_marriage` names its PIDs; and
//! `/close_shop` stays silent with no own shop.
//!
//! Two legacy Defects are not reproduced. `CItem::AddSocket` (`G/item.cpp:1786-1793`) writes
//! `m_alSockets` with neither `UpdatePacket` nor `Save`, so the GM's client keeps showing the
//! old sockets and the store keeps them unless an unrelated later save carries the item; the
//! Rewrite sends the update and stores the row, as `SetSocket` does. And `/set_socket` hands
//! any `int` to `SetSocket`, whose `assert` is the only bound before `m_alSockets[i]` is
//! written, so a socket past the sixth or below zero writes outside the item; the Rewrite
//! writes nothing for it.
//!
//! Not ported: `/set_socket` finds an item held by a character in the world or lying on the
//! ground (`ITEM_MANAGER::Find`, `G/item_manager.cpp:656-662`), but not one in an open
//! safebox or mall, whose rows belong to the account (ADR-0005).

use common::item_slots::{EWindows, INVENTORY_AND_EQUIP_SLOT_MAX, INVENTORY_MAX_NUM};
use common::vid::Vid;
use gamedata::mob_proto::{CHAR_TYPE_MONSTER, CHAR_TYPE_STONE};
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_item_window::ItemAttribute;
use protocol::gc_vid::GcSpecialEffect;
use protocol::item_pos::ItemPos;
use world::character::{
    add_rare_attribute, add_rolled_attribute, change_attribute, change_rare_attribute,
    force_worn_attribute, set_item_socket, ItemChange, MoveDone, MoveKind,
};
use world::item::{ItemId, ATTRIBUTES, SOCKETS};

use super::view::EntityKey;
use super::GameState;
use crate::chat_line::Arg;
use crate::command::{one_argument, two_arguments};
use crate::game_loop_messages::GmLine;
use crate::item_move::{MovedItems, Mover};

/// `WEAR_WEAPON` (4): `GetWear` reads `pItems[INVENTORY_MAX_NUM + cell]`.
const WEAPON_CELL: u16 = INVENTORY_MAX_NUM + common::enums::EWearPositions::Weapon as u16;

/// `MAX_NORM_ATTR_NUM` (`item_length.h:32`): the attributes `ClearAttribute` clears.
const NORM_ATTRS: usize = 5;

/// The rows a GM line changed, which its descriptor's caller stores: the store id of the
/// character whose rows they are, the item changes, and the skills a clear zeroed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmRows {
    /// The PID whose rows changed.
    pub pid: u32,
    /// The stored item row changes, in order.
    pub changes: Vec<db::items::RowChange>,
    /// The skill vnums a clear zeroed that held a level, whose rows are deleted.
    pub cleared_skills: Vec<u8>,
    /// The gold the line's move spent or granted, stored with the rows.
    pub gold: i64,
    /// The monarch spend the line charged: the slot with the landed cell, stored with
    /// the rows. `None` for a line that moved no treasury.
    pub monarch: Option<(u8, i64)>,
    /// The mover's points after the line's move, which the descriptor holds.
    pub points: Option<world::character::Points>,
    /// The mover's quickslots after the line's syncs, which the descriptor holds.
    pub quickslots: Option<world::character::Quickslots>,
}

/// What one attribute line rolled on the worn weapon, for
/// [`GameState::replay_worn_writes`].
struct WornWrites {
    /// The weapon.
    id: ItemId,
    /// The slots cleared first, without a record.
    cleared: std::ops::Range<usize>,
    /// The writes after the clear, in order: each a `SetAttribute` with its update.
    writes: Vec<(usize, ItemAttribute)>,
    /// The attributes before the line and after it.
    attributes: ([ItemAttribute; ATTRIBUTES], [ItemAttribute; ATTRIBUTES]),
    /// Socket 3's new value when the line counts itself (`ChangeAttribute`).
    counted: Option<i32>,
}

/// `str_to_number` on an `int` (`common/utils.h`): `None` for an empty word, which is the
/// only `false` it answers; else `strtol` base 10, garbage reading 0, saturated as `strtol`
/// saturates a `long` before the cast to `int` keeps the low 32 bits of the 32-bit `long`.
pub(super) fn int_word(word: &[u8]) -> Option<i32> {
    if word.is_empty() {
        return None;
    }
    let Some(start) = word.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return Some(0);
    };
    let rest = &word[start..];
    let (digits, negative) = match rest.first() {
        Some(b'-') => (&rest[1..], true),
        Some(b'+') => (&rest[1..], false),
        _ => (rest, false),
    };
    let mut value: i64 = 0;
    for byte in digits.iter().take_while(|byte| byte.is_ascii_digit()) {
        value = (value * 10 + i64::from(byte - b'0')).min(i64::from(i32::MAX) + 1);
    }
    let value = if negative { -value } else { value };
    Some(i32::try_from(value.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0))
}

/// `strtoul(word, NULL, 10)` on the 32-bit target: leading space and one sign skipped,
/// garbage reading 0, a value past `ULONG_MAX` saturating to it, and a negative value
/// negated as an unsigned `long`.
fn unsigned_word(word: &[u8]) -> u32 {
    let Some(start) = word.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return 0;
    };
    let rest = &word[start..];
    let (digits, negative) = match rest.first() {
        Some(b'-') => (&rest[1..], true),
        Some(b'+') => (&rest[1..], false),
        _ => (rest, false),
    };
    let mut value: u64 = 0;
    for byte in digits.iter().take_while(|byte| byte.is_ascii_digit()) {
        value = (value * 10 + u64::from(byte - b'0')).min(u64::from(u32::MAX) + 1);
    }
    let Ok(value) = u32::try_from(value) else {
        return u32::MAX;
    };
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

impl GameState {
    /// Runs one of the five lines for the GM `vid`. Answers the rows a socket line
    /// changed, or `None` when nothing is stored.
    pub(super) fn run_gm_line(
        &mut self,
        vid: Vid,
        line: GmLine,
        rest: &[u8],
        mover: Mover,
    ) -> Option<GmRows> {
        match line {
            GmLine::ItemIdList => {
                self.item_id_list(vid, mover);
                None
            }
            GmLine::MobCount => {
                self.tell_map_mob_count(vid, mover);
                None
            }
            GmLine::Effect => {
                let (word, _) = one_argument(rest);
                // `str_to_number(int)` into `TPacketGCSpecialEffect::type`, a `BYTE`.
                let effect = int_word(&word).unwrap_or(0).to_le_bytes()[0];
                let mut record = Vec::new();
                GcSpecialEffect {
                    effect_type: effect,
                    vid: vid.raw(),
                }
                .encode_into(&mut record);
                self.packet_around(EntityKey::Character(vid.raw()), &record, None);
                None
            }
            GmLine::AddSocket => self.add_socket(vid),
            GmLine::SetSocket => self.set_socket_line(rest),
            GmLine::ChangeAttr | GmLine::AddAttr | GmLine::ChangeRareAttr | GmLine::AddRareAttr => {
                self.roll_weapon_attributes(vid, line)
            }
            GmLine::ForceLogout => {
                // `FindByCharacterName(ch->GetName())` finds the caller's own descriptor,
                // and `DelayedDisconnect(0)` closes it on the next event pass.
                self.order_close(vid);
                None
            }
            GmLine::ResetSubSkill => self.reset_sub_skill(vid, rest, mover),
            GmLine::ConquerorLevel => {
                self.run_conqueror_level(vid, rest, mover);
                None
            }
            GmLine::ConquerorPoint => {
                self.run_conqueror_point(vid, rest, mover);
                None
            }
            GmLine::SortInventory => self.run_sort_inventory(vid, mover),
            GmLine::PickupAffect => {
                self.run_pickup_affect(vid, mover);
                None
            }
            GmLine::MoveChannel => {
                self.run_move_channel(vid, mover, rest);
                None
            }
            GmLine::Flush => {
                self.run_flush(vid, rest);
                None
            }
            GmLine::Observer => {
                self.run_observer(vid);
                None
            }
            GmLine::UserHorseFeed => self.user_horse_feed(vid, mover),
            GmLine::ShowQuiz => self.show_quiz(vid, mover),
            GmLine::GayaSystem => self.run_gaya_command(vid, rest, mover),
            GmLine::Cube => {
                self.cube_command(vid, rest);
                None
            }
            GmLine::FetchCasket => {
                self.fetch_casket_items(vid, rest, mover);
                None
            }
            GmLine::BreakMarriage => {
                self.break_marriage(vid, rest, mover);
                None
            }
            // `do_close_shop` (`G/cmd_general.cpp:2004-2012`): an observer is silent, and a
            // caller with no own shop is silent. No caller of this build holds one
            // (`GetMyShop` is never set, `sys.trade.private_shop` is missing), so the line
            // always stays silent; the arms land with the shop's slice.
            GmLine::CloseShop => None,
            GmLine::CheckMmoney => {
                self.run_check_mmoney(vid, rest, mover);
                None
            }
            GmLine::MonarchWarp => self.run_monarch_warp(vid, rest, mover),
            GmLine::MonarchTransfer => self.run_monarch_transfer(vid, rest, mover),
            GmLine::MonarchInfo => {
                self.run_monarch_info(vid, mover);
                None
            }
            GmLine::MonarchTax => self.run_monarch_tax(vid, rest, mover),
            GmLine::MonarchMob => self.run_monarch_mob(vid, rest, mover),
            GmLine::MonarchNotice => {
                self.run_monarch_notice(vid, rest, mover);
                None
            }
            GmLine::RewardSystem => {
                self.run_reward_system(vid, rest, mover);
                None
            }
        }
    }
    /// `/fetch_casket_items <vnum>` (`do_fetch_casket_items`,
    /// `G/cmd_general.cpp:4136-4173`): an empty line is silent; else the special item
    /// group named by `atoi`'s reading of the word tells one `AddCasketItem <vnum>
    /// <item> <count>` command line per row, in order, and then `SortCasketWindow`.
    /// With no group the quest-registered preview map is read, which is always empty:
    /// no owner quest calls `add_casket_item` (it is `unused`, ledger 401), so the line
    /// stays silent.
    fn fetch_casket_items(&self, vid: Vid, rest: &[u8], mover: Mover) {
        let (word, _) = one_argument(rest);
        if word.is_empty() {
            return;
        }
        let key = int_word(&word).unwrap_or(0);
        let Some(group) = self.item_groups.special.get(&key) else {
            return;
        };
        // `DWORD dwVnum = atoi(arg1)`: the bits of the `int`, as `%u` prints them; the
        // count likewise.
        let shown = u32::from_ne_bytes(key.to_ne_bytes());
        for item in &group.items {
            let count = u32::from_ne_bytes(item.count.to_ne_bytes());
            let line = format!("AddCasketItem {shown} {} {count}", item.vnum);
            self.tell(
                vid,
                mover,
                protocol::gc_chat::CHAT_TYPE_COMMAND,
                line.as_bytes(),
                &[],
            );
        }
        self.tell(
            vid,
            mover,
            protocol::gc_chat::CHAT_TYPE_COMMAND,
            b"SortCasketWindow",
            &[],
        );
    }
    /// `/break_marriage <pid1> <pid2>` (`do_break_marriage`,
    /// `G/cmd_gm.cpp:4162-4174`): the caller hears `Divorce Player %d and Player %d.`
    /// with the words' `strtoul` readings, printed as `%d` prints the `DWORD`s. The DB
    /// divorce then removes the pair from the marriage manager, which holds nobody:
    /// no character of this build is married (`sys.marriage` is missing), so nothing
    /// is stored.
    fn break_marriage(&self, vid: Vid, rest: &[u8], mover: Mover) {
        let (first, second) = two_arguments(rest);
        let pid1 = unsigned_word(&first);
        let pid2 = unsigned_word(&second);
        let args = [
            Arg::Int(i64::from(i32::from_ne_bytes(pid1.to_ne_bytes()))),
            Arg::Int(i64::from(i32::from_ne_bytes(pid2.to_ne_bytes()))),
        ];
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"Divorce Player %d and Player %d.",
            &args,
        );
    }

    /// `/w_gaya <craft|market|refresh> [slot]` (`do_gaya_system`,
    /// `G/cmd_general.cpp:4057-4091`): an empty line is silent, a craft or market with no
    /// slot is silent, and the word runs the craft, the market or the refresh, with the
    /// slot from `atoi`'s reading of the second word. Anything else is silent, as
    /// legacy's `else`-less chain is. The world's moves are sent, and the rows for the
    /// store are answered.
    fn run_gaya_command(&mut self, vid: Vid, rest: &[u8], mover: Mover) -> Option<GmRows> {
        let (first, second) = two_arguments(rest);
        if first.is_empty() {
            return None;
        }
        let (sub_header, pos) = if first == b"craft" {
            if second.is_empty() {
                return None;
            }
            (
                super::gaya_system::GAYA_CRAFT,
                int_word(&second).unwrap_or(0),
            )
        } else if first == b"market" {
            if second.is_empty() {
                return None;
            }
            (
                super::gaya_system::GAYA_MARKET,
                int_word(&second).unwrap_or(0),
            )
        } else if first == b"refresh" {
            (super::gaya_system::GAYA_REFRESH, 0)
        } else {
            return None;
        };
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        let outcome = self.gaya_system(vid, sub_header, pos, mover).ok()?;
        for record in &outcome.records {
            let _sent = self.write_to_client(vid, record.clone());
        }
        Some(GmRows {
            pid,
            changes: outcome.changes,
            cleared_skills: Vec::new(),
            gold: outcome.gold.unwrap_or(0),
            monarch: None,
            points: outcome.points,
            quickslots: outcome.quickslots,
        })
    }

    /// `/flush [pid]` (`do_flush`, `G/cmd_gm.cpp:4425-4446`, the `FLUSH_AT_SHUTDOWN` branch,
    /// which `C/prodomodefines.h:87` defines): an empty line names the caller's own PID, and
    /// a line names `strtoul`'s reading of its first word. A PID other than 0 has the caller
    /// saved at once (`SaveReal`), and the DB process writes its cache of that PID through.
    /// The Rewrite has no cache: every save writes through to the store (ADR-0003), so the
    /// caller's save is all that is left to do.
    fn run_flush(&self, vid: Vid, rest: &[u8]) {
        let (word, _) = one_argument(rest);
        let pid = if word.is_empty() {
            self.characters
                .find_by_vid(vid)
                .map_or(0, world::character::Character::player_id)
        } else {
            unsigned_word(&word)
        };
        if pid == 0 {
            return;
        }
        let Some(clients) = self.clients.as_ref() else {
            return;
        };
        for (channel, id) in clients.leases_of_vid(vid.raw()) {
            let _sent = clients.order(channel, id, crate::client_registry::ClientOrder::Save);
        }
    }

    /// `do_reset_subskill <name>` (`G/cmd_gm.cpp:4315-4334`): an empty line hears the
    /// usage; a name `FindPC` misses hears nothing; else the named character's
    /// `ClearSubSkill` runs at its own level and the caller hears `Subskill of [%s] was
    /// reset`. Answers the target's zeroed skills for the store.
    fn reset_sub_skill(&mut self, vid: Vid, rest: &[u8], mover: Mover) -> Option<GmRows> {
        let (word, _) = one_argument(rest);
        if word.is_empty() {
            let usage = b"Usage: reset_subskill <name>";
            self.tell(vid, mover, CHAT_TYPE_INFO, usage, &[]);
            return None;
        }
        let target = std::str::from_utf8(&word)
            .ok()
            .and_then(|name| self.whisper_target(name))?;
        let level = self.level_of(target)?;
        let cleared_skills = self.clear_sub_skills(target, level);
        let character = self.characters.find_by_vid(target).ok()?;
        let (pid, name) = (character.player_id(), character.name().to_string());
        let line = b"Subskill of [%s] was reset";
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            line,
            &[Arg::Text(name.as_bytes())],
        );
        Some(GmRows {
            pid,
            changes: Vec::new(),
            cleared_skills,
            gold: 0,
            monarch: None,
            points: None,
            quickslots: None,
        })
    }

    /// `do_get_item_id_list`: `cell : %d, name : %s, id : %d` for each item in the
    /// inventory and equipment cells, in cell order.
    /// `/show_quiz` (`do_oxevent_show_quiz`, `G/cmd_oxevent.cpp:11-16`, and
    /// `COXEventManager::ShowQuizList`, `G/OXEvent.cpp:189-204`): the caller hears the
    /// header, each quiz's level, question and answer key (`[LS;858]` for true,
    /// `[LS;869]` for false), and the count (`[LS;880;%d]`), then the footer.
    fn show_quiz(&self, vid: Vid, mover: Mover) -> Option<GmRows> {
        self.tell(vid, mover, CHAT_TYPE_INFO, b"===== OX QUIZ LIST =====", &[]);
        let mut count = 0;
        for level in &self.ox_quiz {
            for quiz in level {
                let end = quiz
                    .question
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(quiz.question.len());
                let key = if quiz.answer {
                    b"[LS;858]".as_slice()
                } else {
                    b"[LS;869]".as_slice()
                };
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"%d %s %s",
                    &[
                        Arg::Int(i64::from(quiz.level)),
                        Arg::Text(&quiz.question[..end]),
                        Arg::Text(key),
                    ],
                );
                count += 1;
            }
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"[LS;880;%d]",
            &[Arg::Int(count)],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"===== OX QUIZ LIST END =====",
            &[],
        );
        None
    }

    fn item_id_list(&self, vid: Vid, mover: Mover) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let lines: Vec<(u16, Vec<u8>, u32)> = (0..INVENTORY_AND_EQUIP_SLOT_MAX)
            .filter_map(|cell| {
                let item = character
                    .items()
                    .item_at(ItemPos::new(EWindows::Inventory as u8, cell))?;
                let name = self
                    .protos
                    .get(item.vnum)
                    .map(|proto| proto.locale_name.clone())
                    .unwrap_or_default();
                Some((cell, name, item.id))
            })
            .collect();
        for (cell, name, id) in lines {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"cell : %d, name : %s, id : %d",
                &[
                    Arg::Int(i64::from(cell)),
                    Arg::Text(&name),
                    // `%d` of the `DWORD` id.
                    Arg::Int(i64::from(i32::from_le_bytes(id.to_le_bytes()))),
                ],
            );
        }
    }

    /// `do_get_mob_count`: the monsters and stones on the caller's map.
    fn tell_map_mob_count(&self, vid: Vid, mover: Mover) {
        let Some((channel, map)) = self.place_of(vid.raw()) else {
            return;
        };
        let count = self.npcs.get(&(channel, map)).map_or(0, |list| {
            list.npcs
                .iter()
                .filter(|npc| {
                    npc.char_type == CHAR_TYPE_MONSTER || npc.char_type == CHAR_TYPE_STONE
                })
                .count()
        });
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"MapIndex: %d MobCount %d",
            &[
                Arg::Int(i64::from(map)),
                Arg::Int(i64::try_from(count).unwrap_or(i64::MAX)),
            ],
        );
    }

    /// `do_add_socket`: the worn weapon's first empty socket takes 1 (`GetSocketCount`,
    /// `G/item.cpp:1776-1784`); a weapon with all six filled, or no weapon, is left alone.
    fn add_socket(&mut self, vid: Vid) -> Option<GmRows> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let weapon = character
            .items()
            .item_at(ItemPos::new(EWindows::Inventory as u8, WEAPON_CELL))?;
        let index = weapon.sockets.iter().position(|&socket| socket == 0)?;
        let id = weapon.id;
        self.write_socket(vid, id, (index, 1))
    }

    /// `do_set_socket <id> <socket> <value>`: three words, each must be present.
    fn set_socket_line(&mut self, rest: &[u8]) -> Option<GmRows> {
        let (first, back) = one_argument(rest);
        let (second, back) = one_argument(back);
        let (third, _) = one_argument(back);
        let id = int_word(&first)?;
        let index = int_word(&second)?;
        let value = int_word(&third)?;
        // `ITEM_MANAGER::Find` takes the `int` as a `DWORD`.
        let id = u32::from_le_bytes(id.to_le_bytes());
        let index = usize::try_from(index)
            .ok()
            .filter(|&index| index < SOCKETS)?;
        let holder = self
            .characters
            .player_names()
            .into_iter()
            .map(|(_, vid)| vid)
            .find(|&vid| {
                self.characters
                    .find_by_vid(vid)
                    .is_ok_and(|character| character.items().item(id).is_some())
            });
        if let Some(holder) = holder {
            return self.write_socket(holder, id, (index, value));
        }
        // A ground item has no owner: `UpdatePacket` sends nothing, and it has no row.
        if let Some(lying) = self
            .ground
            .values_mut()
            .find(|lying| lying.ground.item.id == id)
        {
            let _written = lying.ground.item.set_socket(index, value);
        }
        None
    }

    /// `SetSocket(index, value)` on the item `id` that `holder` holds: its update reaches
    /// the holder's client, and its row is answered for storing.
    fn write_socket(&mut self, holder: Vid, id: ItemId, at: (usize, i32)) -> Option<GmRows> {
        let holder_mover = self.own_mover(holder)?;
        let character = self.characters.find_by_vid_mut(holder).ok()?;
        let pid = character.player_id();
        let written = set_item_socket(character.items_mut(), id, at);
        let moved = MovedItems::new(
            pid,
            holder.raw(),
            MoveDone {
                kind: MoveKind::Socketed,
                records: written.records,
                changes: written.changes,
            },
            holder_mover,
            &self.protos,
            &self.locale,
        );
        for record in moved.records {
            let _sent = self.write_to_client(holder, record);
        }
        Some(GmRows {
            pid,
            changes: moved.changes,
            cleared_skills: Vec::new(),
            gold: 0,
            monarch: None,
            points: None,
            quickslots: None,
        })
    }

    /// The four attribute lines on the worn weapon: `do_change_attr`
    /// (`G/cmd_gm.cpp:3920-3925`, `ChangeAttribute()`), `do_add_attr` (`:3927-3932`,
    /// `AddAttribute()`), `do_change_rare_attr` (`:3942-3947`, `ChangeRareAttribute()`) and
    /// `do_add_rare_attr` (`:3949-3954`, `AddRareAttribute()`). Each clear is silent; each
    /// write after it updates the client's cell, and a normal change of an item that had
    /// attributes then counts itself in socket 3. The points the weapon gives are not
    /// recomputed, as legacy does not, like `/attr_full_set`.
    fn roll_weapon_attributes(&mut self, vid: Vid, line: GmLine) -> Option<GmRows> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let weapon = character
            .items()
            .item_at(ItemPos::new(EWindows::Inventory as u8, WEAPON_CELL))?;
        let (id, before, socket) = (weapon.id, weapon.attributes, weapon.sockets[3]);
        let proto = self.protos.get(weapon.vnum)?;
        let normal = |attributes: &[ItemAttribute]| {
            attributes
                .iter()
                .take(NORM_ATTRS)
                .take_while(|attribute| attribute.b_type != 0)
                .count()
        };
        let prefix = |attributes: &[ItemAttribute], from: usize| -> Vec<(usize, ItemAttribute)> {
            (from..normal(attributes))
                .map(|index| (index, attributes[index]))
                .collect()
        };
        let mut after = before;
        let mut cleared = 0..0;
        let mut counted = None;
        let writes = match line {
            GmLine::ChangeAttr => {
                let attrs = &self.item_attrs;
                if change_attribute(&mut self.dice, &mut self.gauss, proto, &mut after, attrs) {
                    counted = Some(socket.wrapping_add(1));
                }
                cleared = 0..NORM_ATTRS;
                prefix(&after, 0)
            }
            GmLine::AddAttr => {
                add_rolled_attribute(&mut self.dice, proto, &mut after, &self.item_attrs);
                prefix(&after, normal(&before))
            }
            GmLine::ChangeRareAttr => {
                let held = before[NORM_ATTRS..]
                    .iter()
                    .filter(|attribute| attribute.b_type != 0)
                    .count();
                let rare = &self.item_rare_attrs;
                let writes = change_rare_attribute(&mut self.dice, proto, &mut after, rare)?;
                cleared = NORM_ATTRS..NORM_ATTRS + held;
                writes
            }
            GmLine::AddRareAttr => {
                let rare = &self.item_rare_attrs;
                vec![add_rare_attribute(&mut self.dice, proto, &mut after, rare)?]
            }
            _ => return None,
        };
        self.replay_worn_writes(
            vid,
            WornWrites {
                id,
                cleared,
                writes,
                attributes: (before, after),
                counted,
            },
        )
    }

    /// Replays one attribute line on the worn weapon of `vid`: the silent clear, one
    /// `SetAttribute` update per write, and the socket count, then answers the rows.
    fn replay_worn_writes(&mut self, vid: Vid, line: WornWrites) -> Option<GmRows> {
        let mover = self.own_mover(vid)?;
        let character = self.characters.find_by_vid_mut(vid).ok()?;
        let pid = character.player_id();
        let items = character.items_mut();
        let id = line.id;
        let mut done = MoveDone {
            kind: MoveKind::Attributed,
            records: Vec::new(),
            changes: Vec::new(),
        };
        for index in line.cleared {
            let _ = items.set_attribute(id, index, ItemAttribute::default());
        }
        for (index, attribute) in line.writes {
            let written =
                force_worn_attribute(items, id, (index, attribute.b_type, attribute.s_value));
            done.records.extend(written.records);
            done.changes.extend(written.changes);
        }
        let (before, after) = line.attributes;
        if done.changes.is_empty() && after != before {
            // A clear with nothing written back: legacy keeps it in memory unsaved.
            done.changes.push(ItemChange::Attributes {
                id,
                attributes: after,
            });
        }
        if let Some(count) = line.counted {
            let written = set_item_socket(items, id, (3, count));
            done.records.extend(written.records);
            done.changes.extend(written.changes);
        }
        let rolled = MovedItems::new(pid, vid.raw(), done, mover, &self.protos, &self.locale);
        for record in rolled.records {
            let _sent = self.write_to_client(vid, record);
        }
        (!rolled.changes.is_empty()).then_some(GmRows {
            pid,
            changes: rolled.changes,
            cleared_skills: Vec::new(),
            gold: 0,
            monarch: None,
            points: None,
            quickslots: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::int_word;

    #[test]
    fn an_int_word_is_absent_only_when_empty() {
        assert_eq!(int_word(b""), None);
        assert_eq!(int_word(b"junk"), Some(0));
        assert_eq!(int_word(b"12x"), Some(12));
        assert_eq!(int_word(b"-3"), Some(-3));
        assert_eq!(int_word(b"99999999999"), Some(i32::MAX));
        assert_eq!(int_word(b"-99999999999"), Some(i32::MIN));
    }
}
