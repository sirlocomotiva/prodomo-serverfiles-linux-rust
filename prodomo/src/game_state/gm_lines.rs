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
use crate::command::{one_argument, two_arguments, GM_PLAYER};
use crate::game_loop_messages::GmLine;
use crate::item_move::{MovedItems, Mover};

/// `WEAR_WEAPON` (4): `GetWear` reads `pItems[INVENTORY_MAX_NUM + cell]`.
const WEAPON_CELL: u16 = INVENTORY_MAX_NUM + common::enums::EWearPositions::Weapon as u16;

/// `MAX_NORM_ATTR_NUM` (`item_length.h:32`): the attributes `ClearAttribute` clears.
const NORM_ATTRS: usize = 5;

/// `OXEVENT_MAP_INDEX` (`G/OXEvent.h:2`).
const OXEVENT_MAP_INDEX: i32 = 113;

/// The `oxevent_status` event flag (`G/OXEvent.cpp:39-58`), 0 finished.
const OXEVENT_STATUS: &[u8] = b"oxevent_status";

/// The finished status (`OXEVENT_FINISH`), which seats nobody.
const OXEVENT_FINISHED: i32 = 0;

/// The attender spot (`G/OXEvent.cpp:98-101`): the feet that join the attenders.
const OXEVENT_ATTENDER_SPOT: (i32, i32) = (896_500, 24_600);

/// The audience spot (`G/OXEvent.cpp:102-105`): the feet that join the roster only.
const OXEVENT_AUDIENCE_SPOT: (i32, i32) = (896_300, 28_900);

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
pub(super) fn unsigned_word(word: &[u8]) -> u32 {
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
            GmLine::Cube => self.run_cube_command(vid, rest),
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
            GmLine::DungeonInfo => {
                self.run_dungeon_info(vid, rest, mover);
                None
            }
            GmLine::LogOxevent | GmLine::GetOxeventAtt => self.run_ox_line(vid, line, mover),
            _ => self.run_misc_gm_line(vid, line, rest, mover),
        }
    }

    fn run_misc_gm_line(
        &mut self,
        vid: Vid,
        line: GmLine,
        rest: &[u8],
        mover: Mover,
    ) -> Option<GmRows> {
        match line {
            GmLine::ThreewayInfo | GmLine::ThreewayMyinfo => {
                self.run_threeway_line(vid, line, mover)
            }
            GmLine::ShowArenaList
            | GmLine::ArenaDuel
            | GmLine::EndArenaDuel
            | GmLine::EndAllArenaDuel => self.run_arena_line(vid, line, rest, mover),
            GmLine::WeeklyEvent => {
                self.run_weeklyevent_line(vid, rest, mover);
                None
            }
            GmLine::Siege => {
                self.run_siege_line(vid, rest, mover);
                None
            }
            GmLine::Frog => {
                self.run_frog_line(vid, rest, mover);
                None
            }
            GmLine::Worldboss => {
                self.run_worldboss_line(vid, rest, mover);
                None
            }
            GmLine::BattlePass => self.run_battle_pass_gm_line(vid, rest, mover),
            GmLine::BattlePassAdmin => self.run_battle_pass_admin_gm_line(vid, rest, mover),
            _ => None,
        }
    }

    /// `do_cube`'s own step, split out so the dispatcher stays under the line limit.
    fn run_cube_command(&mut self, vid: Vid, rest: &[u8]) -> Option<GmRows> {
        self.cube_command(vid, rest);
        None
    }

    /// The attenders the test reads, pid by pid.
    #[cfg(test)]
    fn ox_attenders(&self) -> &std::collections::HashSet<u32> {
        &self.ox_attenders
    }

    /// The roster the test reads, pid by pid.
    #[cfg(test)]
    fn ox_roster(&self) -> &std::collections::HashSet<u32> {
        &self.ox_roster
    }

    /// The OX roster lines (`do_oxevent_log`, `do_oxevent_get_attender`): the caller's
    /// own choice, split out so the dispatcher stays under the line limit. They change
    /// no row.
    fn run_ox_line(&self, vid: Vid, line: GmLine, mover: Mover) -> Option<GmRows> {
        match line {
            GmLine::LogOxevent => self.log_oxevent(vid, mover),
            GmLine::GetOxeventAtt => self.get_oxevent_att(vid, mover),
            _ => {}
        }
        None
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

    /// `/log_oxevent` (`do_oxevent_log`, `G/cmd_oxevent.cpp:18-28`, and
    /// `COXEventManager::LogWinner`, `G/OXEvent.cpp:442-454`): every attender still
    /// online is char-logged `OXEVENT LastManStanding`, and the caller hears
    /// `[LS;814]`. The `[LS;803]` arm never runs: `LogWinner` always answers true.
    fn log_oxevent(&self, vid: Vid, mover: Mover) {
        let mut attenders: Vec<u32> = self.ox_attenders.iter().copied().collect();
        attenders.sort_unstable();
        for pid in attenders {
            if self.characters.find_by_pid(pid).is_ok() {
                tracing::info!(pid, "CHAR_LOG OXEVENT LastManStanding");
            }
        }
        self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;814]", &[]);
    }

    /// `/get_oxevent_att` (`do_oxevent_get_attender`, `G/cmd_oxevent.cpp:30-33`): the
    /// caller hears the attender count (`[LS;825;%d]`).
    fn get_oxevent_att(&self, vid: Vid, mover: Mover) {
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"[LS;825;%d]",
            &[Arg::Int(
                i64::try_from(self.ox_attenders.len()).unwrap_or(i64::MAX),
            )],
        );
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

impl GameState {
    /// `COXEventManager::Enter`'s seat (`G/OXEvent.cpp:91-104`,
    /// `G/input_login.cpp:853-874`): an entrant on map 113 while the event is not
    /// finished whose feet are exactly the attender spot joins the roster and the
    /// attenders — past the multi-IP kick, which sends a same-host second seat home
    /// instead. The audience spot joins the roster only. A finished event and every
    /// other foot seats nobody, and a refused player (but not a GM) is `WarpSet` to its
    /// empire start.
    pub(super) fn seat_ox_attender(
        &mut self,
        vid: Vid,
        pid: u32,
        place: crate::game_loop_messages::EnterPlace,
    ) {
        if crate::loading_phase::public_map_index(place.map) != OXEVENT_MAP_INDEX {
            return;
        }
        let status = self
            .quests
            .as_ref()
            .map_or(0, |quests| quests.manager().event_flag(OXEVENT_STATUS));
        if status == OXEVENT_FINISHED {
            if self.ox_refuses_player(vid) {
                self.send_ox_kick_home(vid);
            }
            return;
        }
        if (place.x, place.y) == OXEVENT_ATTENDER_SPOT {
            if self.ox_kicks_multi_ip(vid, place.map) {
                return;
            }
            self.ox_roster.insert(pid);
            self.ox_attenders.insert(pid);
            return;
        }
        if (place.x, place.y) == OXEVENT_AUDIENCE_SPOT {
            self.ox_roster.insert(pid);
            return;
        }
        if self.ox_refuses_player(vid) {
            self.send_ox_kick_home(vid);
        }
    }

    /// The `Enter` refusal: a player is `WarpSet` home (`G/input_login.cpp:868-873`), a
    /// GM stays put. A character with no loaded points refuses, as a player does.
    fn ox_refuses_player(&self, vid: Vid) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .is_none_or(|points| points.gm_level() == GM_PLAYER)
    }
}

impl GameState {
    /// `WarpSet(EMPIRE_START_X, EMPIRE_START_Y)` for a refused or kicked OX entrant
    /// (`G/input_login.cpp:868-873`, `G/char.cpp:8616-8619`): the empire start,
    /// ordered through the client registry, as a restart warp is.
    fn send_ox_kick_home(&self, vid: Vid) {
        let Some(body) = self.bodies.get(&vid) else {
            return;
        };
        let Some((x, y)) = crate::warp::go_home(body.card.empire) else {
            return;
        };
        self.order_warp(vid, (x, y));
    }

    /// `COXEventManager::CheckIpAddress` (`G/OXEvent.cpp:119-135`,
    /// `ENABLE_KICK_MULTI_IP_OX`): whether the entrant shares its host with an
    /// attender on the same map — the other must be a live online character, as
    /// legacy's `FindByPID` and `GetDesc` arms require. A kicked entrant is
    /// hack-logged and sent home; the seat does not land.
    fn ox_kicks_multi_ip(&mut self, vid: Vid, map: i32) -> bool {
        let Some(addr) = self.multi_addr.get(&vid).cloned() else {
            return false;
        };
        let hit = self.ox_attenders.iter().any(|pid| {
            let Ok(other) = self.characters.find_by_pid(*pid) else {
                return false;
            };
            let other = other.vid();
            if other == vid {
                return false;
            }
            // `!tch->GetDesc()`: the other must be online, which the outbox proves.
            if !self.outboxes.contains_key(&other) {
                return false;
            }
            if self.bodies.get(&other).is_none_or(|body| body.map != map) {
                return false;
            }
            self.multi_addr
                .get(&other)
                .is_some_and(|other| *other == addr)
        });
        if hit {
            tracing::info!("HACK_LOG MULTI_IP_OX");
            self.send_ox_kick_home(vid);
        }
        hit
    }
}

#[cfg(test)]
mod ox_seat_tests {
    use common::vid::Vid;
    use gamedata::mob_locale_names::MobNamesByLanguage;
    use gamedata::server_attr::SectreeGrid;

    use super::super::fixtures::{a_world, enter_on, TestClock};
    use super::OXEVENT_MAP_INDEX;

    const fn mover() -> crate::item_move::Mover {
        crate::item_move::Mover {
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

    fn text_of(line: &[u8]) -> Vec<u8> {
        let size = usize::from(u16::from_le_bytes([line[1], line[2]]));
        assert_eq!(size, line.len());
        assert_eq!(line[0], 4, "HEADER_GC_CHAT");
        line[10..].to_vec()
    }

    /// The OX map on Channel 1, wide enough for the spots, with a client registry
    /// so the refusal's warp order lands somewhere the test can read.
    fn ox_world(
        clock: &TestClock,
    ) -> (
        super::super::GameState,
        std::sync::Arc<crate::client_registry::ChannelClients>,
    ) {
        let clients = std::sync::Arc::new(crate::client_registry::ChannelClients::new());
        let mut world = a_world(clock).with_clients(std::sync::Arc::clone(&clients));
        world.host_map(
            1,
            OXEVENT_MAP_INDEX,
            SectreeGrid {
                x: 0,
                y: 0,
                columns: 40,
                rows: 40,
            },
        );
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(super::super::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            MobNamesByLanguage::default(),
        ));
        (world, clients)
    }

    /// A seat at `at` for `vid`.
    fn seat(world: &mut super::super::GameState, vid: u32, at: (i32, i32)) {
        world.seat_ox_attender(
            Vid::new(vid),
            vid,
            crate::game_loop_messages::EnterPlace {
                channel: 1,
                map: OXEVENT_MAP_INDEX,
                x: at.0,
                y: at.1,
                z: 0,
            },
        );
    }

    /// A world with the OX event running.
    fn open_world(
        clock: &TestClock,
    ) -> (
        super::super::GameState,
        std::sync::Arc<crate::client_registry::ChannelClients>,
    ) {
        let (mut world, clients) = ox_world(clock);
        world
            .quests
            .as_mut()
            .expect("the quests are installed")
            .manager_mut()
            .set_event_flag(super::OXEVENT_STATUS, 1);
        (world, clients)
    }

    /// `Enter`'s seat: the attender spot joins while the event runs, the audience
    /// spot joins the roster only, and `/get_oxevent_att` counts the one attender.
    /// A same-host second seat kicks home instead of landing.
    #[test]
    fn the_spots_seat_and_the_lines_count_the_attender() {
        let clock = TestClock::default();
        let (mut world, _clients) = open_world(&clock);
        let mut seater = enter_on(
            &mut world,
            (1, OXEVENT_MAP_INDEX),
            7,
            (896_500, 24_600),
            820,
        );
        let mut onlooker = enter_on(
            &mut world,
            (1, OXEVENT_MAP_INDEX),
            8,
            (896_300, 28_900),
            820,
        );
        seat(&mut world, 7, (896_500, 24_600));
        seat(&mut world, 8, (896_300, 28_900));
        assert!(
            world.ox_roster().contains(&8),
            "the audience joins the roster"
        );
        assert!(
            !world.ox_attenders().contains(&8),
            "the audience is no attender"
        );
        // The enter walk's own lines (multi-farm, registry) land before the count.
        while seater.try_recv().is_ok() {}
        while onlooker.try_recv().is_ok() {}
        world.get_oxevent_att(Vid::new(7), mover());
        let count = seater.try_recv().expect("the count answers");
        assert_eq!(text_of(&count), b"[LS;825;1]");
        seat(&mut world, 8, (896_500, 24_600));
        world.get_oxevent_att(Vid::new(7), mover());
        let count = seater.try_recv().expect("the count answers again");
        assert_eq!(text_of(&count), b"[LS;825;1]");
        // `/log_oxevent` answers `[LS;814]`; the char-log is the operator log's own.
        world.log_oxevent(Vid::new(7), mover());
        let logged = seater.try_recv().expect("the log answers");
        assert_eq!(text_of(&logged), b"[LS;814]");
        // A finished event refuses a later seat: the count stays 1.
        world
            .quests
            .as_mut()
            .expect("the quests are installed")
            .manager_mut()
            .set_event_flag(super::OXEVENT_STATUS, 0);
        let mut late = enter_on(
            &mut world,
            (1, OXEVENT_MAP_INDEX),
            9,
            (896_500, 24_600),
            820,
        );
        while late.try_recv().is_ok() {}
        seat(&mut world, 9, (896_500, 24_600));
        while seater.try_recv().is_ok() {}
        world.get_oxevent_att(Vid::new(7), mover());
        let count = seater
            .try_recv()
            .expect("the count answers after the late seat");
        assert_eq!(text_of(&count), b"[LS;825;1]");
    }

    /// A finished event refuses: a GM is sent nowhere, and a warp keeps the seat
    /// while a disconnect from map 113 drops it (`G/char.cpp:1827-1830`).
    #[test]
    fn a_finished_event_refuses_and_the_leave_drops_the_seat() {
        let clock = TestClock::default();
        let (mut world, clients) = ox_world(&clock);
        let mut seater = enter_on(
            &mut world,
            (1, OXEVENT_MAP_INDEX),
            7,
            (896_500, 24_600),
            820,
        );
        while seater.try_recv().is_ok() {}
        let mut points = super::super::fixtures::points(820);
        points.set_gm_level(crate::command::GM_IMPLEMENTOR);
        world
            .characters
            .find_by_vid_mut(Vid::new(7))
            .expect("the GM is in the world")
            .set_points(Some(points));
        // The lease joins the registry's map-113 roster before the finished seat, as
        // the descriptor's does — so the seat's own order would land here.
        let mut lease = clients.join(crate::client_registry::ClientEntry {
            channel: 1,
            map: OXEVENT_MAP_INDEX,
            name: "P7".to_string(),
            vid: 7,
            empire: 1,
            language: 0,
        });
        seat(&mut world, 7, (896_500, 24_600));
        let mut orders = lease.take_orders().expect("the queue is home");
        assert!(orders.try_recv().is_err(), "a refused GM is sent nowhere");
        // The seat never landed; the leave drops nothing and keeps nothing.
        world.leave_world_by(Vid::new(7), crate::game_loop_messages::Departure::Warp);
        assert!(world.ox_attenders().is_empty(), "no seat ever landed");
        world.leave_world_by(
            Vid::new(7),
            crate::game_loop_messages::Departure::Disconnect,
        );
        assert!(world.ox_roster().is_empty(), "no roster ever landed");
        // The running event seats, and the leave keeps the seat on a warp and drops
        // it with the roster on a disconnect from map 113.
        world
            .quests
            .as_mut()
            .expect("the quests are installed")
            .manager_mut()
            .set_event_flag(super::OXEVENT_STATUS, 1);
        let mut rider = enter_on(
            &mut world,
            (1, OXEVENT_MAP_INDEX),
            8,
            (896_500, 24_600),
            820,
        );
        while rider.try_recv().is_ok() {}
        seat(&mut world, 8, (896_500, 24_600));
        assert!(world.ox_attenders().contains(&8), "the seater sits");
        world.leave_world_by(Vid::new(7), crate::game_loop_messages::Departure::Warp);
        assert!(world.ox_attenders().contains(&8), "a warp keeps the seat");
        world.leave_world_by(
            Vid::new(8),
            crate::game_loop_messages::Departure::Disconnect,
        );
        assert!(
            !world.ox_attenders().contains(&8),
            "a disconnect from map 113 drops the seat"
        );
        assert!(
            !world.ox_roster().contains(&8),
            "a disconnect from map 113 drops the roster too"
        );
    }
}
