//! The quest manager: what an NPC click runs, the state machine of a running script and the
//! dialog it sends (ADR-0004, ADR-0006).
//!
//! It ports legacy's `CQuestManager` (`G/questmanager.cpp`), `quest::NPC` (`G/questnpc.cpp`),
//! `quest::PC` (`G/questpc.cpp`) and the running half of `G/questlua.cpp`:
//!
//! - [`Manager::new`] indexes the compiled scripts of the [`Host`] by NPC, event, quest and
//!   state, as `CQuestManager::Initialize` (`questmanager.cpp:70-172`) and `NPC::Set`
//!   (`questnpc.cpp:28-187`) did: the NPCs `questnpc.txt` names, `notarget` as NPC 0, and then
//!   every directory of `object/` named by a number, which legacy's `RegisterNPCVnum` added for
//!   every monster and item it loaded;
//! - [`Manager::click`] is `CQuestManager::Click` (`questmanager.cpp:918-990`): an NPC with
//!   `chat` scripts offers the menu of the entries whose condition holds, and runs its `click`
//!   scripts when it offers none or has no `chat` scripts;
//! - a script runs in a Lua thread (`OpenState`, `RunState`, `questlua.cpp:970-1056`) and
//!   suspends on `select`, `wait` and `input`, each of which sends the dialog built so far
//!   through [`Game::script`], as `SendScript` (`questmanager.cpp:1113-1151`) sent `GC_SCRIPT`;
//! - [`Manager::answer`] is `CG_SCRIPT_ANSWER` (`CInputMain::ScriptAnswer`), which picks a menu
//!   entry or continues a `wait`, and [`Manager::input`] is `CG_QUEST_INPUT_STRING`;
//! - a script that set its quest's title or called `q.start` or `q.done` sends the quest's
//!   window through [`Game::quest_info`] when it ends, as `EndRunning` sent `GC_QUEST_INFO`
//!   (`questpc.cpp:231-308`, `:343-346`), and [`Manager::info`] runs the `info` script the
//!   quest window asks for;
//! - a script that moved its quest to another state runs, as it ends, the old state's `leave`
//!   script, the new state's `enter` script and, when that left the state alone, the new
//!   state's `letter` script (`EndRunning`, `questpc.cpp:352-371`).
//!
//! While the manager runs, the [`crate::host::BRIDGED`] names reach the game through
//! [`crate::host::BRIDGE`] and a [`Game`] the caller lends for that one call.
//!
//! # Divergences
//!
//! - The dialog text, its skin and the error flag live for one call into the manager. Legacy
//!   kept them process-wide, so text a script added and never sent went out with the next
//!   character's dialog; the Rewrite logs and drops it.
//! - A script error, a suspension other than `select`, `wait`, `input`, `confirm` and
//!   `select_item`, a first result that is not text (legacy's `strcmp(NULL)`)
//!   a `select` whose last result is not a table (legacy's unprotected `luaL_getn`) all end
//!   the script the way a Lua error did.
//! - A call of an API the Rewrite does not back ends the script as a script that says nothing
//!   ends: the bare `[DONE]` is not sent (`SendScript`, `G/questmanager.cpp:1122-1130`). The
//!   legacy's API worked, and its script carried on to an end the client saw nothing of, so
//!   the refusal sends nothing either. A script error still ends the script the legacy's error
//!   did, with the `[DONE]` the client closes the window with.
//! - A condition (`when ... with`, `begin_condition`) that raises an error is false, and so is a
//!   chat menu label that raises one empty: legacy read the error message as the result, which
//!   made a failing condition true.
//! - The chunks keep their `object/` names in error messages, not the quest's name.
//! - `questnpc.txt` is required; a sign before a number ends the file, as any other non-digit
//!   does, and an empty name is logged and skipped.
//! - The files of an event are read in byte order, not in `readdir` order.
//! - A chat menu index outside `0..=65535` is logged and skipped (legacy allocates it).
//! - A file whose quest has no number is logged and skipped (legacy asserts), and a quest number
//!   with no quest names the empty quest.
//! - Quest states and flags live in the store with the character (ledger 244). `pc.getqf`
//!   reads them (ledger 257), `pc.setqf` writes them (ledger 259) and
//!   `pc.learn_grand_master_skill` writes its read count; no other API reaches them yet.
//! - State changes nest at most 16 deep (`STATE_CHANGE_DEPTH`). An `enter` script that moved its
//!   quest on ran the next change inside the first, so two states whose `enter` scripts named
//!   each other recursed in legacy until the stack ran out; the Rewrite logs the change past
//!   the limit and runs none of its scripts. No owner quest nests a change at all.
//! - `getnpcid` with no name answers 0 (legacy built a `std::string` from `NULL`).
//! - An NPC's race is looked up by the caller, which finds the NPC on the character's own map.
//! - A `questcategory.txt` line with fewer than two words is logged and skipped; legacy read
//!   past the end of its word list.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use mlua::{Function, Lua, MultiValue, Thread, Value};

use crate::host::{self, Host, HostError, BRIDGE};

/// `QUEST_SKIN_NOWINDOW`: the client closes the dialog.
pub const QUEST_SKIN_NOWINDOW: u8 = 0;

/// `QUEST_SKIN_NORMAL`: the dialog window, the skin every dialog starts with.
pub const QUEST_SKIN_NORMAL: u8 = 1;

/// `CONFIRM_NO` (`G/quest.h:83-88`): the confirm's script hears 0 for a no.
pub const CONFIRM_NO: u8 = 0;

/// `CONFIRM_YES`: the confirm's script hears 1 for a yes.
pub const CONFIRM_YES: u8 = 1;

/// `CONFIRM_TIMEOUT`: the confirm's script hears 2 when nobody answered in time.
pub const CONFIRM_TIMEOUT: u8 = 2;

/// `QUEST_SKIN_COUNT` (`questmanager.h:47-55`): `set_skin` takes a skin below it.
const QUEST_SKIN_COUNT: u8 = 6;

/// The event directories of `object/` (`questmanager.cpp:80-109`, with `__EVENT_MANAGER__` and
/// `ENABLE_QUEST_DIE_EVENT` defined), in the order `NPC::Set` walks its name map.
pub const EVENTS: [&str; 26] = [
    "button",
    "chat",
    "click",
    "die",
    "enter",
    "event_begin",
    "event_end",
    "in",
    "info",
    "item_informer",
    "kill",
    "leave",
    "letter",
    "levelup",
    "login",
    "logout",
    "out",
    "party_kill",
    "pick",
    "server_timer",
    "sig_use",
    "take",
    "target",
    "timer",
    "unmount",
    "use",
];

/// The quest a chat menu runs under (`NPC::OnChat`); it has no number, so it is quest 0.
const CHAT_TEMP_QUEST: &[u8] = b"QUEST_CHAT_TEMP_QUEST";

/// `QUEST_NO_NPC` (`quest.h:18`): the NPC index of the scripts no NPC owns.
const QUEST_NO_NPC: u32 = 0;

/// `MAIN_RACE_MAX_NUM` (`G/char.h:62-73`): the eight player races; a kill of one of them
/// runs only the no-target scripts.
const MAIN_RACE_MAX_NUM: u32 = 8;

/// The largest chat menu index the loader accepts.
const MAX_ARG_INDEX: i64 = 0xffff;

/// A `CG_SCRIPT_ANSWER` above this continues a `wait`; one at or below it picks a menu entry.
const ANSWER_RESUME: u8 = 250;

/// How deep one state change may run another inside its scripts before the Rewrite stops it
/// (the module's Divergences).
const STATE_CHANGE_DEPTH: u8 = 16;

/// The chat line types the quest API sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatKind {
    /// `CHAT_TYPE_TALKING`, from `chat`.
    Talking,
    /// `CHAT_TYPE_INFO`, from `syschat`.
    Info,
    /// `CHAT_TYPE_NOTICE`, from `notice`.
    Notice,
    /// `CHAT_TYPE_COMMAND`, from `cmdchat`.
    Command,
}

/// What a running script reaches in the game, for the character it runs for.
pub trait Game {
    /// `GetLevel()` of the character the script runs for.
    fn level(&self) -> u8;

    /// `GetRealPoint(POINT_PLAYTIME)` of the character the script runs for: the whole minutes
    /// it has played, as the character's last save banked them.
    fn playtime(&self) -> i32;

    /// `ITEM_MANAGER::GetVnum` (`G/item_manager.cpp:722-738`): the vnum of the first proto
    /// whose locale name `name` prefixes, ignoring case.
    fn item_vnum(&self, name: &[u8]) -> Option<u32>;

    /// `AutoGiveItem(vnum, count)` for the character the script runs for
    /// (`G/char_item.cpp:9081-9223`): the ID of the item it gave or topped up last, or 0
    /// when it gave none.
    fn give_item(&mut self, vnum: u32, count: u16) -> u32;

    /// `CountSpecifyItem(vnum)` of the character the script runs for
    /// (`G/char_item.cpp:8791-8846`): how many of `vnum` its inventory holds.
    fn count_item(&self, vnum: u32) -> i32;

    /// `RemoveSpecifyItem(vnum, count)` of the character the script runs for
    /// (`G/char_item.cpp:8848-8940`): `count` of `vnum` taken from its inventory, stack by
    /// stack in cell order, or all it holds when that is less.
    fn remove_item(&mut self, vnum: u32, count: u32);

    /// `GetSkillGroup()` of the character the script runs for.
    fn skill_group(&self) -> u8;

    /// `GetJob()` of the character the script runs for.
    fn job(&self) -> u8;

    /// The vnum of the item of ID `id` the character the script runs for holds, or `None`
    /// when it holds none: what `item.get_vnum` reads through the quest item pointer.
    fn held_vnum(&self, id: u32) -> Option<u32>;

    /// The cell of the item of ID `id` the character the script runs for holds, or `None`
    /// when it holds none: what `item.get_cell` reads through the quest item pointer
    /// (`CItem::GetCell`, `G/item.h:116`).
    fn held_cell(&self, id: u32) -> Option<u16>;

    /// The ID of the item in the character's base inventory cell `cell`, or `None` when
    /// the cell holds none: what `item.select_cell` moves the quest item pointer to
    /// (`CHARACTER::GetInventoryItem`, `G/char_item.cpp:249-252`).
    fn inventory_at(&self, cell: u32) -> Option<u32>;

    /// `CItem::SetSocket(index, value)` (`G/item.cpp:1643-1656`) of the item of ID `id`, the
    /// quest item, with `index` below the six sockets: the item the character holds is updated
    /// in its cell and saved, and one its give laid on the ground takes the value unseen.
    fn set_socket(&mut self, id: u32, index: usize, value: i32);

    /// The race of the character `vid`, when it is in the world.
    fn race_of(&self, vid: u32) -> Option<u32>;

    /// The name of item `vnum` in the character's language, when `ITEM_MANAGER::GetTable` finds
    /// its proto.
    fn item_name(&self, vnum: u32) -> Option<Vec<u8>>;

    /// The name of monster `vnum` in the character's language, when the monster exists.
    fn mob_name(&self, vnum: u32) -> Option<Vec<u8>>;

    /// Sends the character a chat line whose text is `text`.
    fn chat(&mut self, kind: ChatKind, text: &[u8]);

    /// Sends the character a `GC_SCRIPT`.
    fn script(&mut self, skin: u8, script: &[u8]);

    /// `get_global_time()`: the wall clock in seconds.
    fn global_time(&self) -> u32;

    /// `number(from, to)`: a random number from `from` to `to`.
    fn number(&mut self, from: i32, to: i32) -> i32;
    /// `mining::OreRefine(ch, npc, item, cost, pct, metinstone)` (`G/mining.cpp:421-464`):
    /// the quest item of ID `ore` melts into its refined vnum, 100 of its count for one,
    /// for the fee `cost` with the refine NPC's guild rate, and `Some` of the inventory
    /// cell `stone`'s item (the melt's detritus) goes with it. Answers `false` with no
    /// NPC, when the character holds no such quest item, when the ore's count is below
    /// 100, when no refined vnum answers its vnum, or when the gold does not cover the
    /// fee; the fee and the metin go only when the melt runs, and the refined item lands
    /// through `AutoGiveItem` on the draw (`number(1, 100) <= pct`).
    fn ore_refine(&mut self, ore: u32, cost: i32, pct: i32, stone: Option<u32>) -> bool;

    /// `LearnGrandMasterSkill(vnum)` (`G/char_skill.cpp:341`): the 7/8th-skills
    /// quest's read of a GRAND skill (230g). `total` is the read count including
    /// this one, which the bridge already stamped in the quest flags. Answers
    /// whether the skill climbed.
    fn learn_grand_master_skill(&mut self, vnum: u32, total: i32) -> bool;

    /// `GetSkillLevel(vnum)` of the character the script runs for (230g).
    fn skill_level(&self, vnum: u32) -> u8;

    /// `SetSkillLevel(vnum, level)` then `SkillLevelPacket()` of the character the script
    /// runs for (`G/questlua_pc.cpp:2112-2123`): the slot takes the level, up to 40, unless
    /// the vnum is past the slots or a 7/8th skill's exclusion refuses it, and the client
    /// hears every slot either way.
    fn set_skill_level(&mut self, vnum: u32, level: u8);

    /// `GetRealAlignment()` of the character the script runs for
    /// (`G/questlua_pc.cpp:1081-1085`), before the /10 the client sees.
    fn real_alignment(&self) -> i32;

    /// `UpdateAlignment(amount)` of the character the script runs for
    /// (`G/questlua_pc.cpp:1093-1100`, `G/char_battle.cpp:3477-3565`): the stored
    /// alignment moves by `amount`, the crossing update goes out, and the band affects
    /// follow.
    fn change_alignment(&mut self, amount: i32);

    /// `FindAffect(AFFECT_SKILL_NO_BOOK_DELAY)` of the character the script runs for
    /// (`G/questlua_pc.cpp:113-119`): whether it holds the no-delay book affect.
    fn has_skill_book_no_delay(&self) -> bool;

    /// `RemoveAffect(AFFECT_SKILL_NO_BOOK_DELAY)` of the character the script runs for
    /// (`G/questlua_pc.cpp:106-111`).
    fn remove_skill_book_no_delay(&mut self);

    /// `SetBlockModeForce(mode)` of the character the script runs for
    /// (`G/questlua_pc.cpp:2190-2197`): the byte becomes the character's own
    /// block mode, the client hears `setblockmode %d`, and no quest flag moves.
    fn send_block_mode(&mut self, mode: u8);

    /// `GetMapIndex()` of the character the script runs for: the map it stands on, the
    /// private index when it stands on a private map.
    fn map_index(&self) -> i32;

    /// `GetWarMap()` of the character the script runs for: the war map's private index,
    /// and 0 when it stands on no war map.
    fn war_map(&self) -> i32;
    /// `GetGold()` of the character the script runs for (`G/questlua_pc.cpp:1073-1077`).
    fn gold(&self) -> u64;
    /// `pc_change_money` (`G/questlua_pc.cpp:1102-1121`) under its four names: the gold
    /// moves by `(long long)` of the last argument, unless the move would drop it below
    /// 0 (logged, nothing moves) or reach `GOLD_MAX_MAX` (logged, nothing moves).
    fn change_gold(&mut self, amount: i64);
    /// `pc_hide_costume` (`G/questlua_pc.cpp:3553-3600`): part 1-5 (body, hair, sash, aura,
    /// weapon) hides or shows, the client hears `SetXCostumeHidden`, and the parts the
    /// next update sends follow.
    fn hide_costume(&mut self, part: u8, hide: bool);
    /// `pc_open_sash` (`G/questlua_pc.cpp:3523-3536`): the sash window opens for
    /// combination (`true`) or absorption, unless the window already stands (the
    /// legacy `SASH_OPEN`/`SASH_CLOSE`/`COMBINE_CLOSE` line chats instead).
    fn open_sash(&mut self, combination: bool);
    /// `pc_open_changelook` (`G/questlua_pc.cpp:3540-3549`): the changelook window
    /// opens, unless it already stands (the transmutation line chats instead).
    fn open_changelook(&mut self);
    /// `pc_mining` (`G/questlua_pc.cpp:1881-1887`): `CHARACTER::mining` on the NPC the script
    /// runs for, `npc_vid` (0 with none): a running dig is cancelled, else the dig starts
    /// when the NPC is a vein near the character and a pick is worn.
    fn mining(&mut self, npc_vid: u32);
    /// `_refine_pick` and `_fish_real_refine_rod` (`G/questlua_global.cpp:450-478`):
    /// `RealRefinePick` (or, with `rod`, `RealRefineRod`) on the tool in inventory `cell`;
    /// answers 1 for the refined tool, 0 for the fallback and 2 for a refusal.
    fn refine_tool(&mut self, cell: u8, rod: bool) -> u8;

    /// The horse of the character the script runs for (`CHorseRider`), as the `horse.*` and
    /// `pc.*_horse_*` readers see it.
    fn horse(&self) -> HorseView;

    /// `SetHorseLevel`, `ComputePoints` and `SkillLevelPacket` on the character the script
    /// runs for (`G/questlua_horse.cpp:110-121`); the level is already clamped to 0..30.
    fn set_horse_level(&mut self, level: i32);

    /// `FeedHorse` on the character the script runs for (`G/questlua_horse.cpp:231-240`).
    fn feed_horse(&mut self);

    /// `horse.summon` (`G/questlua_horse.cpp:72-90`): `HorseSummon(true, from_far, vnum)` on
    /// the character the script runs for, unless it rides a mount or stands on the OX map.
    /// The world runs it when the step ends.
    fn summon_horse(&mut self, from_far: bool, vnum: u32);

    /// `horse.unsummon` (`G/questlua_horse.cpp:92-97`): `HorseSummon(false)`, run when the
    /// step ends.
    fn unsummon_horse(&mut self);

    /// `ReviveHorse` (`horse.revive`, `pc.revive_horse`), run when the step ends.
    fn revive_horse(&mut self);

    /// `horse.ride` (`G/questlua_horse.cpp:53-64`): `StartRiding`, run when the step
    /// ends, unless it rides a mount or stands on the OX map or an arena.
    fn ride_horse(&mut self);

    /// `horse.unride` (`G/questlua_horse.cpp:66-71`): `StopRiding`, run when the step
    /// ends.
    fn unride_horse(&mut self);

    /// `horse.get_name` (`G/questlua_horse.cpp:273-291`): the manager's name for the
    /// character the script runs for, empty with none.
    fn horse_name(&self) -> Vec<u8>;

    /// `pc.mount(vnum, seconds)` (`G/questlua_pc.cpp:1392-1458`): the mount affect for
    /// `seconds`, run when the step ends.
    fn mount(&mut self, vnum: u32, seconds: i32);

    /// `pc.unmount` (`G/questlua_pc.cpp:1481-1489`), run when the step ends.
    fn unmount(&mut self);

    /// `pc.mount_bonus(apply, value, seconds)` (`G/questlua_pc.cpp:1461-1479`), run when
    /// the step ends.
    fn mount_bonus(&mut self, apply: u8, value: i32, seconds: i32);

    /// `GetAttr67AddItem()` of the character the script runs for
    /// (`G/char_item.cpp:10474-10477`): the vnum the window holds, 0 with none.
    fn attr67_vnum(&self) -> u32;

    /// The rare attribute onto the window's item (`CItem::AddRareAttribute`,
    /// `G/item_attribute.cpp:422-464`): its cell is updated and its row is saved.
    fn attr67_add_rare(&mut self);

    /// The window's item into the first free base cell (`attr67add.collect`,
    /// `G/questlua_attr67add.cpp:46-71`): the old cell clears, the new one fills, and
    /// the row follows it. Answers whether it moved; with no room nothing moves.
    fn attr67_collect(&mut self) -> bool;

    /// `pc.global_rank_add_point(type, count)` (`G/questlua_pc.cpp:3723-3736`): `count`
    /// onto counter `kind` of the character the script runs for, run when the step ends.
    fn global_rank_add_point(&mut self, kind: u8, count: i32);

    /// `pc.global_rank_get_reward` (`G/questlua_pc.cpp:3680-3721`): every prize the character
    /// won and has not claimed, each given by `AutoGiveItem`. Answers whether it had any.
    fn global_rank_get_reward(&mut self) -> bool;

    /// `npc.is_pc` (`G/questlua_npc.cpp:32-41`): whether the character under `vid` is a PC.
    fn npc_is_pc(&self, vid: u32) -> bool;

    /// What `npc.is_metin` and `npc.is_boss` read of the character under `vid`
    /// (`G/questlua_npc.cpp:400-428`): whether it is a stone and its `GetMobRank()`, or
    /// `None` with no such character.
    fn npc_stone_and_rank(&self, vid: u32) -> Option<(bool, u8)>;

    /// `pc.in_dungeon` (`G/questlua_pc.cpp:318-323`): whether the character the script runs
    /// for is in a dungeon.
    fn in_dungeon(&self) -> bool;

    /// `horse.set_name` (`G/questlua_horse.cpp:231-271`): names the character's horse
    /// `name` — the flag, the month affect and the re-summon around the rewrite — and
    /// answers 2, 1 for a bad name, 0 without a horse. The world runs it when the step
    /// ends; `name` holds no zero byte.
    fn set_horse_name(&mut self, name: Vec<u8>) -> u8;

    /// `GetEmpire()` of the character the script runs for (`G/questlua_pc.cpp:1304-1308`).
    fn empire(&self) -> u8;

    /// `GetHP()` of the character the script runs for (`G/questlua_pc.cpp:975-979`).
    fn hp(&self) -> i32;

    /// `GetMaxHP()` of the character the script runs for (`G/questlua_pc.cpp:969-973`).
    fn max_hp(&self) -> i32;

    /// `GetSP()` of the character the script runs for (`G/questlua_pc.cpp:927-931`).
    fn sp(&self) -> i32;

    /// `GetMaxSP()` of the character the script runs for (`G/questlua_pc.cpp:921-925`).
    fn max_sp(&self) -> i32;

    /// `GetExp()` of the character the script runs for (`G/questlua_pc.cpp:897-901`).
    fn exp(&self) -> u32;

    /// `GetNextExp()` of the character the script runs for (`G/questlua_pc.cpp:891-895`).
    fn next_exp(&self) -> u32;

    /// `GetRaceNum()` of the character the script runs for (`G/questlua_pc.cpp:903-907`).
    fn race(&self) -> u16;

    /// `GetName()` of the character the script runs for (`G/questlua_pc.cpp:885-889`).
    fn char_name(&self) -> Vec<u8>;

    /// The VID of the character the script runs for (`G/questlua_pc.cpp:880-884`).
    fn char_vid(&self) -> u32;

    /// `GetLeadershipSkillLevel()` of the character the script runs for
    /// (`G/questlua_pc.cpp:862-866`): its leadership skill's level.
    fn leadership(&self) -> u8;

    /// `GetX()/100` and `GetY()/100` of the character the script runs for
    /// (`G/questlua_pc.cpp:737-749`), and the local pair against the map's base
    /// (`:751-775`).
    fn position(&self) -> (i32, i32, i32, i32);

    /// `GetWear(cell)` of the character the script runs for (`G/questlua_pc.cpp:1052-1071`):
    /// the vnum worn in `cell`, and `None` when the cell is out of range or empty.
    fn worn_vnum(&self, cell: u8) -> Option<u32>;

    /// `GetEmpire()` of the NPC `vid` (`G/questlua_npc.cpp:43-52`), and `None` when no NPC
    /// stands under it.
    fn empire_of(&self, vid: u32) -> Option<u8>;
    /// `GetGuild()->GetID()` of the NPC `vid` (`G/questlua_npc.cpp:60-70`), and 0 when no
    /// NPC stands under it or it holds no guild. Only a building's NPC holds one
    /// (`CObject::RegenNPC`, `G/building.cpp:268-298`).
    fn npc_guild(&self, vid: u32) -> u32;
    /// `CManager::FindLand(map, x, y)->GetID()` (`G/questlua_building.cpp:12-29`): the
    /// first land in id order holding `(x, y)` on `map` after the map-base shift, and
    /// 0 with none.
    fn land_id_at(&self, map: i32, x: i32, y: i32) -> u32;
    /// `MeleyLair_Register` (`G/questlua_MeleyLair.cpp:37-49`): the register's
    /// (`iRes1`, `iRes2`) pair. The shadow judges from the step's reads; the world
    /// judges the queued call again after the step.
    fn meley_register(&mut self) -> (i32, i32);
    /// `MeleyLair_IsRegistered` (`G/questlua_MeleyLair.cpp:51-68`): whether the
    /// reader's guild is registered, and the channel that registered it.
    fn meley_is_registered(&self) -> (bool, u8);
    /// `MeleyLair_Enter` (`G/questlua_MeleyLair.cpp:70-87`): whether the reader
    /// enters, and the pushed limit when it does not. The world judges the queued
    /// call again after the step, which warps the reader in.
    fn meley_enter(&mut self) -> (bool, i32);
    /// `MeleyLair_IsMeleyMap` (`G/questlua_MeleyLair.cpp:89-97`): whether the reader
    /// stands on Meley's map (212 or its 2120000..2130000 private copies).
    fn is_meley_map(&self) -> bool;
    /// `MeleyLair_Check` (`G/questlua_MeleyLair.cpp:99-107`): the login check is
    /// queued for the world after the step.
    fn meley_check(&mut self);
    /// `MeleyLair_Leave` (`G/questlua_MeleyLair.cpp:109-117`): the leave is queued
    /// for the world after the step.
    fn meley_leave(&mut self);
    /// `MeleyLair_CanGetReward` (`G/questlua_MeleyLair.cpp:119-131`): whether the
    /// reader may take the chest — a step-4 participant past the chest's clock who
    /// took no reward yet.
    fn meley_can_reward(&self) -> bool;
    /// `MeleyLair_Reward` (`G/questlua_MeleyLair.cpp:133-145`): the argument's
    /// chest is queued for the world after the step, which rows the take and gives
    /// the chest.
    fn meley_reward(&mut self, arg: u8);
    /// `ship_defense_mgr_create` (`G/questlua_shipdefense_mgr.cpp:28-42`): queued for
    /// the world after the step, which stands the run and warps the reader in.
    fn ship_create(&mut self);
    /// `ship_defense_mgr_start` (`G/questlua_shipdefense_mgr.cpp:44-62`): queued for
    /// the world after the step, which runs the deck.
    fn ship_start(&mut self);
    /// `ship_defense_mgr_join` (`G/questlua_shipdefense_mgr.cpp:64-74`): queued for
    /// the world after the step, which warps the reader in.
    fn ship_join(&mut self);
    /// `ship_defense_mgr_leave` (`G/questlua_shipdefense_mgr.cpp:76-86`): queued for
    /// the world after the step, which warps the reader out.
    fn ship_leave(&mut self);
    /// `ship_defense_mgr_land` (`G/questlua_shipdefense_mgr.cpp:88-98`): queued for
    /// the world after the step, which warps the reader to the port.
    fn ship_land(&mut self);
    /// `ship_defense_mgr_is_created` (`G/questlua_shipdefense_mgr.cpp:100-111`):
    /// whether the reader's leaders' run stands unstarted.
    fn ship_is_created(&self) -> bool;
    /// `ship_defense_mgr_is_running` (`G/questlua_shipdefense_mgr.cpp:113-124`):
    /// whether the reader's leaders' run has started.
    fn ship_is_running(&self) -> bool;
    /// `ship_defense_mgr_set_alliance_hp_pct`
    /// (`G/questlua_shipdefense_mgr.cpp:142-159`): queued for the world after the
    /// step, which heals the mast, chats the HP and stuns the reader.
    fn ship_heal(&mut self, pct: u8);
    /// `ship_defense_mgr_can_join` (`G/questlua_shipdefense_mgr.cpp:161-174`):
    /// whether the reader's party leader holds a run.
    fn ship_can_join(&self) -> bool;
    /// `building_get_land_info`'s triple (`G/questlua_building.cpp:31-58`): the
    /// `(price, owner, level limit)` of `land`, and `None` with no such land.
    fn land_info(&self, land: u32) -> Option<(u32, u32, u8)>;
    /// `CManager::FindLandByGuild(guild) != NULL` (`G/questlua_building.cpp:83-118`):
    /// whether `guild` owns a land. The store lookup answers from memory.
    fn guild_has_land(&self, guild: u32) -> bool;
    /// `building_set_land_owner` (`G/questlua_building.cpp:60-80`): land `land` goes to
    /// `guild` when it exists and stands unowned; the world judges and stores after
    /// the step.
    fn set_land_owner(&mut self, land: u32, guild: u32);
    /// `building_reconstruct` (`G/questlua_building.cpp:126-153`): the object whose NPC
    /// is `npc_vid` is rebuilt as `vnum`, when the object's land is guild-owned and
    /// `vnum` names a prototype.
    fn reconstruct_object(&mut self, npc_vid: u32, vnum: u32);
    /// `GetPlayerID()` of the character the script runs for (`G/questlua_pc.cpp:2608-2621`).
    fn player_id(&self) -> u32;

    /// `GetDesc()->GetAccountTable().id` of the character the script runs for
    /// (`G/questlua_pc.cpp:2624-2638`): its account's ID, 0 with no descriptor. The
    /// Rewrite always has the descriptor's account table, so the ID answers.
    fn account_table_id(&self) -> u32;

    /// `GetGMLevel()` of the character the script runs for (`G/questlua_pc.cpp:1867-1877`):
    /// its `m_pointsInstant.gm_level`, which rode the admission. The test-server silence
    /// (`G/char.cpp:6355-6356`) is the audit's (V26): the production arm answers.
    fn gm_level(&self) -> u8;

    /// `GetEmptyInventory(size) != -1` for a `vnum` with a proto
    /// (`G/questlua_pc.cpp:478-499`, `G/char_item.cpp:1258-1267`): whether a fresh base
    /// cell below the unlocked count is empty, and false with no proto.
    fn has_empty_inventory(&self, vnum: u32) -> bool;

    /// `GetSocket(index)` of the item of ID `id` (`G/questlua_item.cpp:107-123`): the
    /// quest item's socket, and `None` when the character holds no such item.
    fn held_socket(&self, id: u32, index: usize) -> Option<i32>;

    /// `GetValue(index)` of the item of ID `id` (`G/questlua_item.cpp:175-202`): the
    /// quest item's proto value, and `None` when the character holds no such item or
    /// no proto names its vnum.
    fn held_value(&self, id: u32, index: usize) -> Option<i32>;

    /// `GetCount()`, `GetType()` and `GetSubType()` of the item of ID `id`
    /// (`G/questlua_item.cpp:259-301`): the quest item's count and its proto's type
    /// pair, and `None` when the character holds no such item. The type pair is 0
    /// with no proto (`CItem::GetType`, `G/item.h:108-109`).
    fn held_shape(&self, id: u32) -> Option<(u16, u8, u8)>;

    /// `GetName()`, `GetSize()` and `GetRefinedVnum()` of the item of ID `id`
    /// (`G/questlua_item.cpp:233-310`): the quest item's locale name, its proto's size
    /// and refined vnum, and `None` when the character holds no such item. The name
    /// is empty with no proto (`CItem::GetName`, `G/item.h:64-66`); the numbers are 0
    /// (`GetSize`, `GetRefinedVnum`, `G/item.h:69,181`).
    fn held_refine(&self, id: u32) -> Option<(Vec<u8>, u8, u32)>;

    /// The refine level the item of ID `id` reads (`CItem::GetRefineLevel`,
    /// `G/item.cpp:1869-1892`): the number after the last `+` in its proto's `szName`.
    /// `None` when the character holds no such item; 0 with no proto or no `+`.
    fn held_refine_level(&self, id: u32) -> Option<i32>;

    /// `ITEM_MANAGER::RemoveItem` on the item of ID `id` (`G/questlua_item.cpp:91-105`):
    /// the quest item the character holds leaves its window and its cell clears, with
    /// the quickslot and store rows a removal makes. One the character holds nowhere
    /// takes nothing.
    fn remove_quest_item(&mut self, id: u32);

    /// `GetGuild()` of the character the script runs for (`G/questlua_pc.cpp:325-336`): its
    /// guild id, and 0 with none.
    fn guild_id(&self) -> u32;

    /// Whether the character the script runs for leads its guild (`G/questlua_pc.cpp:339-350`).
    fn is_guild_master(&self) -> bool;

    /// `CGuildManager::FindGuild` (`G/guild_manager.cpp:224`): whether a guild holds `id`.
    /// The observer warp's `no such guild` line asks it.
    fn guild_known(&self, id: u32) -> bool;

    /// `test_server` (`G/config.cpp:72`): whether the deployment runs in test-server mode.
    fn test_server(&self) -> bool;

    /// `DESC::GetLanguage` (`G/desc.h:182`): the descriptor's language byte, which `LC`
    /// reads (`G/questlua_global.cpp:44-54`).
    fn language(&self) -> u8;

    /// `_notice_all` (`G/questlua_global.cpp:944-991`): the at most six `parts` reach
    /// every online character as a notice, formatted and quest-translated as
    /// `SendLocaleNotice` does. The world judges after the step.
    fn notice_all(&mut self, parts: Vec<Vec<u8>>);

    /// `temple_ochao_initialize` (`G/questlua_TempleOchao.cpp:15-20`): the temple's
    /// rooms arm and its guardian spawns. The world judges after the step.
    fn prepare_temple_ochao(&mut self);

    /// `_do_command` (`G/questlua_global.cpp:730-741`): `interpret_command` runs `line`
    /// as the character. The world judges after the step.
    fn quest_command(&mut self, line: Vec<u8>);

    /// `_add_ox_quiz` (`G/questlua_global.cpp:1144-1156`): the quiz `question` at
    /// `level` with `answer` lands in the OX list (`COXEventManager::AddQuiz`,
    /// `G/OXEvent.cpp:175-186`). The world judges after the step.
    fn add_ox_quiz(&mut self, level: u8, question: Vec<u8>, answer: bool);

    /// `game_open_gaya_c` (`G/questlua_game.cpp:95-101`): the client opens the Gaya
    /// window. The world judges after the step.
    fn open_gaya(&mut self);

    /// `game_open_gaya_m` (`G/questlua_game.cpp:103-126`): the Gaya market markers are
    /// refreshed and the client opens the Gaya market. The world judges after the step.
    fn open_gaya_market(&mut self);

    /// `game_open_aura_absorb_window` (`G/questlua_game.cpp:312-323`): the absorb
    /// window opens on the NPC `npc_vid` (0 with none). The world judges after the
    /// step.
    fn open_aura_absorb(&mut self, npc_vid: u32);

    /// `game_open_aura_growth_window` (`G/questlua_game.cpp:326-337`): the growth
    /// window opens on the NPC `npc_vid` (0 with none). The world judges after the
    /// step.
    fn open_aura_growth(&mut self, npc_vid: u32);

    /// `game_open_aura_evolve_window` (`G/questlua_game.cpp:340-351`): the evolve
    /// window opens on the NPC `npc_vid` (0 with none). The world judges after the
    /// step.
    fn open_aura_evolve(&mut self, npc_vid: u32);

    /// `_start_event` (`G/questlua_global.cpp:1636-1665`): the event `index` starts
    /// with `value0` and `value1`. The world judges after the step.
    fn start_event(&mut self, index: i32, value0: i32, value1: i32);

    /// `_end_event` (`G/questlua_global.cpp:1668-1686`): the event `index` ends. The
    /// world judges after the step.
    fn end_event(&mut self, index: i32);

    /// `_find_pc` (`G/questlua_global.cpp:743-757`): the VID of the online PC named `name`,
    /// case-insensitively, and 0 when no PC holds it.
    fn find_pc_vid(&self, name: &[u8]) -> u32;

    /// The PID of the character `vid` names, when a PC stands under it
    /// (`CHARACTER_MANAGER::Find`, `G/questlua.cpp:922-926`): the confirm's target.
    fn pid_of(&self, vid: u32) -> Option<u32>;

    /// `CHARACTER::ConfirmWithMsg` (`G/char.cpp:8076-8088`): the `GC_QUEST_CONFIRM` with
    /// `msg` and `timeout` for the character `to_pid`, naming `request_pid`. Sent at once,
    /// as `ConfirmWithMsg` sends it; the quest that asked keeps its own script.
    fn confirm_to(&mut self, to_pid: u32, msg: &[u8], timeout: i32, request_pid: u32);

    /// Arms the confirm's timeout: after `timeout` seconds the waiter's script hears
    /// `CONFIRM_TIMEOUT` (`confirm_timeout_event`, `G/questlua.cpp:879-904`).
    fn arm_confirm_timeout(&mut self, waiter: u32, timeout: i32);

    /// `pc_destroy_guild` (`G/questlua_pc.cpp:352-370`): the character's guild disbands,
    /// unless it holds none, it is at war (the caller hears the line), or the character
    /// does not lead it. The world judges after the step.
    fn destroy_guild(&mut self);

    /// `pc_remove_from_guild` (`G/questlua_pc.cpp:372-381`): the character leaves its
    /// guild, unless it holds none or leads it (both quiet). The world judges after the
    /// step.
    fn remove_from_guild(&mut self);
    /// `game.request_make_guild` (`G/questlua_game.cpp:51-62`): the character's client hears
    /// the one-byte `HEADER_GC_REQUEST_MAKE_GUILD`.
    fn request_make_guild(&mut self);

    /// One `guild.*` quest call (230s): the call's member name (`level`, `get_sp0`) and
    /// its arguments, dworded numbers or raw text. The world's guilds judge it now, while
    /// the script that asked still waits, and answer what legacy pushed.
    fn guild_call(&mut self, name: &[u8], args: &[GuildArg]) -> GuildAnswer;

    /// Sends the character a `GC_QUEST_INFO`: the window of the quest whose script just
    /// ended (`quest::PC::SendQuestInfoPakcet`, `G/questpc.cpp:231-308`).
    fn quest_info(&mut self, info: &QuestInfo);

    /// `_find_npc_by_vnum` (`G/questlua_global.cpp:792-823`): the VID of a character of race
    /// `race` on the character's map, among the races `CHARACTER_MANAGER` registers, and 0
    /// when there is none.
    fn find_npc(&self, race: u32) -> u32;

    /// `CTargetManager::CreateTarget` (`G/target.cpp:134-238`) with `TARGET_TYPE_VID`: the
    /// character's quest target `target`, which replaces the one its quest gave the same name.
    fn create_target(&mut self, target: &Target);

    /// `CTargetManager::DeleteTarget` (`G/target.cpp:240-282`): the character's targets of
    /// quest `quest` named `name`, or all of them when `name` is `None`, end.
    fn delete_target(&mut self, quest: u32, name: Option<&[u8]>);

    /// `GetTargetEvent` (`G/target.cpp:284-315`): the VID the character's target of quest
    /// `quest` named `name` points at, and 0 when there is no such target.
    fn target_vid(&self, quest: u32, name: &[u8]) -> u32;

    /// `GetTargetInfo(pid, TARGET_TYPE_VID, vid)` (`G/target.cpp:317-348`): the quest and the
    /// kept name of the character's first target that points at `vid`, when one does.
    fn target_at(&self, vid: u32) -> Option<(u32, Vec<u8>)>;

    /// `pc_warp` (`G/questlua_pc.cpp:209-240`): the character warps to (`x`, `y`) on
    /// `private` (a private map index at or above 10000, else the position's own map),
    /// unless `IsHack` refuses. Answers whether the warp runs. The test-server line stays
    /// with the audit, as the anywhere spawn's does.
    fn warp(&mut self, x: i32, y: i32, private: i32) -> bool;

    /// `pc_warp_to_guild_war_observer_position`
    /// (`G/questlua_pc.cpp:406-443`): the character joins the war between the two guilds
    /// as an observer — the flag cleared, the exit remembered, and the warp to the
    /// war map's observer position. Answers the position when the war has one, so the
    /// bridge can tell a missing guild from a guildless war; the `luaL_error` lines are
    /// the bridge's. Quiet under `IsHack`, like `pc_warp`.
    fn war_observe(&mut self, first: u32, second: u32) -> Option<(i32, i32, i32)>;

    /// `game_drop_item_with_ownership` (`G/questlua_game.cpp:168-213`): `count` of `vnum`
    /// lie at the character's feet, owned by it for `secs` seconds (`None` with no
    /// number: 30, as 10 or below). Nothing happens with no proto.
    fn drop_with_ownership(&mut self, vnum: u32, count: i32, secs: Option<i32>);

    /// `pc_get_conqueror_level` (`G/questlua_pc.cpp:3632-3637`): the conqueror level of
    /// the character the script runs for.
    fn conqueror_level(&self) -> u8;

    /// `GetConquerorExp()` of the character the script runs for
    /// (`G/questlua_pc.cpp:3617-3622`).
    fn conqueror_exp(&self) -> u32;

    /// `_warp_to_village` (`G/questlua_global.cpp:1060-1071`): the character warps to
    /// its empire's start. It pushes nothing.
    fn warp_to_village(&mut self);

    /// `pc_can_warp` (`G/questlua_pc.cpp:2723-2737`): whether `CanWarp` passes for the
    /// character the script runs for.
    fn can_warp(&self) -> bool;

    /// `pc_set_conqueror` (`G/questlua_pc.cpp:3639-3644`, always `true`; `G/char.cpp`,
    /// `SetConqueror`): the character becomes a conqueror past the level-120 marble
    /// gates. It pushes nothing.
    fn set_conqueror(&mut self);

    /// `GetMountVnum()` of the character the script runs for
    /// (`G/questlua_pc.cpp:1385-1390`): whether it rides a mount.
    fn is_mount(&self) -> bool;

    /// `GetParty()` of the character the script runs for
    /// (`G/questlua_party.cpp:236-241`): whether it stands in a party.
    fn is_party(&self) -> bool;

    /// `GetParty()->GetLeaderPID() == GetPlayerID()` of the character the
    /// script runs for (`G/questlua_party.cpp:224-234`): whether it leads its
    /// party.
    fn is_party_leader(&self) -> bool;

    /// `g_bChannel` (`G/questlua_pc.cpp:2767-2772`): the Channel the script runs
    /// on.
    fn channel_id(&self) -> u8;

    /// `GetParty()->GetFlag(name)` of the character the script runs for
    /// (`G/questlua_party.cpp:315-325`): its party's flag, 0 with no party.
    fn party_flag(&self, name: &[u8]) -> i32;

    /// `GetParty()->SetFlag(name, value)` of the character the script runs for
    /// (`G/questlua_party.cpp:305-313`): its party's flag becomes `value`.
    /// Quiet with no party.
    fn set_party_flag(&mut self, name: Vec<u8>, value: i32);

    /// `GetParty()->GetLeaderPID()` of the character the script runs for
    /// (`G/questlua_party.cpp:243-255`): its party's leader PID, -1 with none.
    fn party_leader_pid(&self) -> i32;

    /// The PIDs of the party members on the character's map, in ascending PID
    /// order (`FPartyPIDCollector` over `ForEachOnMapMember`,
    /// `G/questlua_party.cpp:413-442`, `G/party.h:295-309`): the PIDs
    /// `party.get_member_pids` pushes. Empty with no party.
    fn party_member_pids(&self) -> Vec<u32>;

    /// `GetParty()->GetNearMemberCount()` of the character the script runs for
    /// (`G/questlua_party.cpp:195-204`): its party's cached near count, 0 with no
    /// party. The count is the last Update's; the call itself updates nothing.
    fn party_near_count(&self) -> u32;

    /// `CQuestManager::BeginOtherPCBlock(pid)`
    /// (`G/questmanager.cpp:2051-2076`): the character `pid` names becomes the
    /// one the script runs for, and the bridge answers for it until
    /// [`Game::end_other_pc`] runs. Quiet with no such online character: the
    /// stack still grows, but the script keeps running for the old one.
    fn begin_other_pc(&mut self, pid: u32);

    /// Whether the last [`Game::begin_other_pc`] switched the character: false
    /// when the named character is not online, so the script kept running for
    /// the old one (`GetPC` returned `NULL`, `G/questmanager.cpp:1087-1099`).
    fn switched_pc(&self) -> bool;

    /// `CQuestManager::EndOtherPCBlock` (`G/questmanager.cpp:2078-2093`): the
    /// script runs for the character it ran for before the matching
    /// [`Game::begin_other_pc`] again. Quiet with no open block.
    fn end_other_pc(&mut self);

    /// `CDungeonManager::FindByMapIndex(index) != NULL`
    /// (`G/questlua_dungeon.cpp:1540-1563`): whether a dungeon lives under the
    /// map index.
    fn dungeon_lives(&self, index: u32) -> bool;
    /// `CDungeonManager::Create(map)` (`G/dungeon.cpp:470-501`) then the party's jump:
    /// `d.new_jump_party` (`G/questlua_dungeon.cpp:407-432`) for the character the script
    /// runs for. The leader's on-map party jumps to `map`'s first free private index at
    /// (`x`, `y`); quiet with a bad map, a taken dungeon set, or no party.
    fn dungeon_new_jump_party(&mut self, map: i32, x: i32, y: i32);
    /// `CDungeonManager::Create(map)` (`G/dungeon.cpp:470-501`) then the reader's jump:
    /// `d.new_jump` (`G/questlua_dungeon.cpp:318-348`) for the character the script
    /// runs for. The reader alone jumps to `map`'s first free private index at
    /// (`x`, `y`); quiet with a bad map, a taken dungeon set, or no reader.
    fn dungeon_new_jump(&mut self, map: i32, x: i32, y: i32);
    /// `d.get_map_index` of the script's dungeon (`G/questlua_dungeon.cpp:184-201`):
    /// its private map index, 0 with none.
    fn dungeon_map_index(&self) -> i32;
    /// `d.setf` on the script's dungeon (`G/questlua_dungeon.cpp:106-130`): `name`
    /// takes `value`. Quiet with none.
    fn dungeon_set_flag(&mut self, name: Vec<u8>, value: i32);
    /// `d.getf` on the script's dungeon (`G/questlua_dungeon.cpp:131-153`): the flag
    /// `name`, 0 with none or no such flag.
    fn dungeon_get_flag(&self, name: &[u8]) -> i32;
    /// `d.getf_from_map_index` (`G/questlua_dungeon.cpp:155-182`): the flag `name`
    /// of the dungeon under `index`, 0 with none, no such dungeon, or no such flag.
    fn dungeon_flag_from_index(&self, name: &[u8], index: u32) -> i32;
    /// `d.notice` to the script's dungeon (`G/questlua_dungeon.cpp:52-86`,
    /// `G/dungeon.cpp:1154-1176`): the notice `text` to its PCs. Quiet with none.
    fn dungeon_notice(&mut self, text: Vec<u8>);
    /// `d.set_warp_location` on the script's dungeon (`G/questlua_dungeon.cpp:1603-1622`):
    /// every member PC's pending warp becomes (`map`, `x`, `y`). Quiet with none.
    fn dungeon_set_warp_location(&mut self, map: i32, x: i32, y: i32);
    /// `d.exit_all` on the script's dungeon (`G/questlua_dungeon.cpp:1257-1266`,
    /// `G/dungeon.cpp:1109-1123`): every PC on its map exits to its saved place.
    /// Quiet with none.
    fn dungeon_exit_all(&mut self);
    /// `d.spawn_mob` in the script's dungeon (`G/questlua_dungeon.cpp`,
    /// `G/dungeon.cpp:762-781`): `vnum` at (`x`, `y`). Quiet with none.
    fn dungeon_spawn_mob(&mut self, vnum: u32, x: i32, y: i32);
    /// `d.clear_regen` on the script's dungeon (`G/questlua_dungeon.cpp:235-243`,
    /// `G/dungeon.cpp:895-904`): its regens go. Quiet with none.
    fn dungeon_clear_regen(&mut self);
    /// `d.kill_all` on the script's dungeon (`G/questlua_dungeon.cpp:1491-1499`,
    /// `G/dungeon.cpp:996-1008`): every non-player on its map dies. Quiet with
    /// none.
    fn dungeon_kill_all(&mut self);
    /// `d.set_damage_from_race` (`G/questlua_dungeon.cpp:902-919`): the character
    /// under `vid` refuses damage from attackers of race `flag`. No dungeon is
    /// needed: legacy finds the character by VID.
    fn dungeon_set_damage_from_race(&mut self, flag: u32, vid: u32);
    /// `d.set_damage_from_affect` (`G/questlua_dungeon.cpp:921-938`): the character
    /// under `vid` takes damage only from attackers holding affect flag `flag`.
    /// No dungeon is needed: legacy finds the character by VID.
    fn dungeon_set_damage_from_affect(&mut self, flag: u32, vid: u32);
    /// `d.select`'s adoption (`G/questmanager.cpp:1252-1264`): the session's dungeon
    /// becomes `index`, or 0 with `None`. The bridge calls it only while no
    /// character runs (PID 0), because the selection is ignored with one; a fresh
    /// step arms 0.
    fn dungeon_select(&mut self, index: Option<i32>);
    /// `game.set_event_flag` (`G/questlua_game.cpp:29-37`): the flag `name` takes
    /// `value`. Quiet unless the name is a string and the value a number, which the
    /// bridge checks.
    fn set_event_flag(&mut self, name: Vec<u8>, value: i32);
    /// `server_timer(name, secs, arg)` (`G/questlua_global.cpp:147-171`): the
    /// `server_timer` event fires in `secs` seconds with `arg`.
    fn set_server_timer(&mut self, name: Vec<u8>, secs: f64, arg: u32);
    /// `clear_server_timer(name, arg)` (`G/questlua_global.cpp:197-206`): the
    /// named timer with `arg` never fires.
    fn clear_server_timer(&mut self, name: &[u8], arg: u32);

    /// `DragonSoul_IsQualified()` of the character the script runs for
    /// (`G/questlua_dragonsoul.cpp:57-70`): whether it holds
    /// `AFFECT_DRAGON_SOUL_QUALIFIED` (540, `G/affect.h:105`).
    fn is_ds_qualified(&self) -> bool;

    /// `DragonSoul_GiveQualification()` of the character the script runs for
    /// (`G/questlua_dragonsoul.cpp:43-55`, `G/char_dragonsoul.cpp:75-85`):
    /// `AddAffect(540, APPLY_NONE, 0, AFF_NONE, INFINITE_AFFECT_DURATION, 0,
    /// false)` (`G/char_affect.cpp:668-760`). Quiet with no character.
    fn give_ds_qualification(&mut self);

    /// `ds_open_refine_window` of the character the script runs for
    /// (`G/questlua_dragonsoul.cpp:14-28`): with the qualification, its client
    /// hears `HEADER_GC_DRAGON_SOUL_REFINE` with `DS_SUB_HEADER_OPEN`.
    fn open_ds_refine(&mut self);

    /// `ds_open_attr_change_window` of the character the script runs for
    /// (`G/questlua_dragonsoul.cpp:30-41`, `G/char.cpp:11235-11242`): its client
    /// hears `HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN`.
    fn open_ds_attr_change(&mut self);

    /// `IsPolymorphed()` of the character the script runs for
    /// (`G/questlua_pc.cpp:1361-1366`, `G/char.h:901`): whether a morph is on.
    fn is_polymorphed(&self) -> bool;

    /// `GetGlobalRank` (`G/char_cards.cpp:336-369`): the Okey table's summed rank text,
    /// empty with no ended game.
    fn okay_global_rank(&self) -> Vec<u8>;

    /// `GetRundRank` (`G/char_cards.cpp:371-403`): the Okey table's round rank text,
    /// empty with no ended game.
    fn okay_round_rank(&self) -> Vec<u8>;

    /// `IsEngaged(player_id)` of the character the script runs for
    /// (`G/questlua_pc.cpp:1846-1851`): whether it is engaged. No character of
    /// this build is (`sys.marriage` is missing), so this answers false.
    fn is_engaged(&self) -> bool;

    /// `IsMarried(player_id)` of the character the script runs for
    /// (`G/questlua_pc.cpp:1853-1858`): whether it is married. No character of
    /// this build is (`sys.marriage` is missing), so this answers false.
    fn is_married(&self) -> bool;

    /// `CHARACTER::ChangeEmpire(empire)` (`G/char_change_empire.cpp:23-129`) for the
    /// character the script runs for: 1 for the empire it holds, 4 in a party, 2 with
    /// a guilded character on the account, 3 with an engaged or married one (never:
    /// no character of this build is), and 999 when the move lands. The world judges
    /// it now, while the script that asked still waits, and the move's records land
    /// beside the script's own. The store move and the count follow after the step.
    fn change_empire(&mut self, empire: u8) -> i32;

    /// `CHARACTER::GetChangeEmpireCount()` (`G/char_change_empire.cpp:131-162`): how
    /// many times the account changed its empire, which the world read at the step's
    /// start.
    fn change_empire_count(&self) -> u32;

    /// `LogManager::CharLog(ch, what, how, hint)` (`G/log.cpp:110-121`): the quest log
    /// row. It is server-side only, so the Rewrite logs the line and pushes nothing.
    fn char_log(&mut self, what: u32, how: &[u8], hint: &[u8]);

    /// `_syserr` (`G/questlua_global.cpp:385-403`): the server log line `QUEST: quest:
    /// <quest> player: <name> : <text>` and the `QUEST_SYSERR <text>` info chat.
    fn sys_err(&mut self, quest: &[u8], text: &[u8]);

    /// `CountEmptyInventory()` of the character the script runs for
    /// (`G/questlua_pc.cpp:2576-2590`, `G/char_item.cpp:1331-1347`): the unlocked cells
    /// minus the sizes of the items anchored in them.
    fn empty_inventory_count(&self) -> u32;

    /// `GetExchange() != NULL` (`G/questlua_pc.cpp:3379-3384`) of the character the
    /// script runs for: whether it trades.
    fn is_trade0(&self) -> bool;

    /// `GetExchange() || GetMyShop() || GetShopOwner() || IsOpenSafebox() || IsCubeOpen()`
    /// (`G/questlua_pc.cpp:3386-3392`) of the character the script runs for: whether any
    /// of its windows stands open. The private shop and the cube are not ported, so
    /// those two never answer true.
    fn is_busy0(&self) -> bool;

    /// `pc_give_exp2(exp)` (`G/questlua_pc.cpp:1245-1261`): `PointChange(POINT_EXP, exp)`
    /// on the character the script runs for. It pushes nothing, and the rewards log is
    /// not ported (as `pc.give_item2`'s). Legacy's `exp` is `(DWORD)rint` of the first
    /// argument; a non-number runs nothing.
    fn give_exp2(&mut self, exp: u32);

    /// `pc_set_skillgroup(group)` (`G/questlua_pc.cpp:1345-1358`): the good affects go,
    /// then `SetSkillGroup` (`G/char_skill.cpp:149-165`) runs — a group past 2 or below
    /// level 5 changes nothing. It pushes nothing. The group's record lands after the
    /// step, with the skill rows the descriptor stores.
    fn set_skill_group(&mut self, group: u8);

    /// `pc_warp_local(index, x, y)` (`G/questlua_pc.cpp:242-316`): the character warps to
    /// the region's base plus (`x`, `y`), unless the map index names no region or a
    /// coordinate stands past the region's width. It pushes nothing; a refusal only logs.
    /// The commented window checks never ran, so no window refuses.
    fn warp_local(&mut self, index: i32, x: i32, y: i32);

    /// `pc_select_vid(vid)` (`G/questlua_pc.cpp:1814-1836`): the script runs for the
    /// character under `vid` from now on, and the old one's VID is pushed; with none
    /// under it, nothing switches and 0 is pushed.
    fn select_pc(&mut self, vid: u32) -> u32;

    /// `pc_get_killee_drop_pct()` (`G/questlua_pc.cpp:2962-2980`): `GetDropPct` of the
    /// NPC under `npc_vid` against the character the script runs for. With no such NPC
    /// on the step's map, `(-1, -1)`.
    fn killee_drop_pct(&mut self, npc_vid: u32) -> (i32, i32);

    /// `game_drop_item(vnum, count)` (`G/questlua_game.cpp:138-166`): `count` of `vnum`
    /// lie on the ground at the character's feet, unowned, with the 300-second destroy
    /// event. It pushes nothing; with no proto nothing is made.
    fn drop_item(&mut self, vnum: u32, count: i32);
}

/// The horse a quest reads (`CHorseRider`'s getters).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HorseView {
    /// `GetHorseLevel`, 0 to 30.
    pub level: u8,
    /// `GetHorseHealth`.
    pub health: i16,
    /// `GetHorseStamina`.
    pub stamina: i16,
    /// `GetHorseMaxHealth`: the level's row, 1 without a horse.
    pub max_health: i16,
    /// `GetHorseMaxStamina`: the level's row, 1 without a horse.
    pub max_stamina: i16,
    /// `IsHorseRiding`.
    pub riding: bool,
    /// The VID of the summoned horse (`GetHorse()`), 0 with none.
    pub summoned: u32,
}

impl HorseView {
    /// `GetHorseGrade` (`G/horse_rider.cpp:378-386`).
    #[must_use]
    pub const fn grade(&self) -> u8 {
        if self.level == 0 {
            0
        } else {
            (self.level - 1) / 10 + 1
        }
    }
}

/// What `target.vid` asks `CTargetManager::CreateTarget` for (`G/questlua_target.cpp:45-66`):
/// a target on a character.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// `GetCurrentQuestIndex()`: the number of the quest that runs, 0 for the chat menu's.
    pub quest: u32,
    /// The name the quest knows the target by, up to its first zero.
    pub name: Vec<u8>,
    /// The VID the target points at.
    pub vid: u32,
    /// The name the client shows, up to its first zero, when the script gave one.
    pub desc: Option<Vec<u8>>,
    /// `iSendFlag`: whether the client hears of the target.
    pub send: bool,
}

/// What `SendQuestInfoPakcet` (`G/questpc.cpp:231-308`) sends when a script that changed its
/// quest's window ends: the quest's number and category, and the parts the script changed.
/// The icon is not ported, so no script changes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestInfo {
    /// `iIndex`, the low half the record's `WORD` keeps: the number of the quest that ran.
    pub index: u16,
    /// `GetQuestCategoryByQuestIndex(index)`: the category `questcategory.txt` gives the
    /// quest, 0 when it gives none.
    pub category: u16,
    /// `bStart`, when `q.start` or `q.done` set `QUEST_SEND_ISBEGIN`.
    pub begin: Option<bool>,
    /// `_title`, when `q.set_title` set `QUEST_SEND_TITLE`: the text up to its first zero.
    pub title: Option<Vec<u8>>,
    /// `_clock_name`, when `q.set_clock_name` set `QUEST_SEND_CLOCK_NAME`.
    pub clock_name: Option<Vec<u8>>,
    /// `_clock_value`, when `q.set_clock_value` set `QUEST_SEND_CLOCK_VALUE`.
    pub clock_value: Option<i32>,
    /// `_counter_name`, when `q.set_counter_name` set `QUEST_SEND_COUNTER_NAME`.
    pub counter_name: Option<Vec<u8>>,
    /// `_counter_value`, when `q.set_counter_value` set `QUEST_SEND_COUNTER_VALUE`.
    pub counter_value: Option<i32>,
}

/// One argument of a `guild.*` quest call: a dworded number or the raw bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuildArg {
    /// `(DWORD)lua_tonumber`, in the world's units.
    Number(u32),
    /// `lua_tostring`, up to its first zero byte.
    Text(Vec<u8>),
}

/// What a `guild.*` quest call answers: what legacy pushed, as values.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub enum GuildAnswer {
    /// Nothing pushed (`return 0`).
    None,
    /// `lua_pushnumber`.
    Number(f64),
    /// `lua_pushboolean`.
    Boolean(bool),
    /// `lua_pushstring`.
    Text(Vec<u8>),
    /// A `lua_newtable` of rows, each row's numbers in index order
    /// (`guild.get_reserve_war_table`, `guild.get_warp_war_list`).
    Table(Vec<Vec<f64>>),
}

/// A compiled chunk, or why it could not be compiled; the error ends the script that runs it.
type Code = mlua::Result<Function>;

/// Every quest, NPC and running script.
#[derive(Debug)]
pub struct Manager {
    host: Host,
    npcs: BTreeMap<u32, Npc>,
    npc_ids: BTreeMap<Vec<u8>, u32>,
    event_flags: BTreeMap<Vec<u8>, i32>,
    pcs: BTreeMap<u32, Pc>,
    /// `m_dwQuestNPCVID` (`G/char.h:1772`): the NPC the last `SetQuestNPCID` named. The
    /// bridge reads it when the character's own click did not name one.
    quest_npc_vid: u32,
    /// `QuestCategoryIndexMap` (`G/questmanager.h:308`): each quest's category, by number.
    categories: BTreeMap<u16, u32>,
    /// `m_mapTimerID` (`G/questmanager.cpp:1202-1218`): each server-timer name's NPC id,
    /// counted down from `u32::MAX`.
    timer_ids: BTreeMap<Vec<u8>, u32>,
    /// `m_dwServerTimerArg` (`G/questmanager.cpp:2014-2017`): the firing timer's
    /// argument, which `get_server_timer_arg` pushes.
    timer_arg: u32,
    /// `m_pSelectedDungeon` (`G/questmanager.cpp:1252-1264`): the private index
    /// `d.select` named, which the `d.*` names read when no character runs.
    selected: Option<u32>,
}

impl Manager {
    /// Loads the [`Host`] of the locale directory `locale_dir` and indexes its scripts with
    /// `quest/questnpc.txt`, and its quests' categories with `quest/questcategory.txt`.
    ///
    /// A `questcategory.txt` that cannot be read is logged and gives every quest category 0,
    /// as `ReadQuestCategoryToDict` did (`G/questmanager.cpp:320-331`).
    ///
    /// # Errors
    ///
    /// The host fails to load, or `questnpc.txt` cannot be read.
    pub fn load(locale_dir: &Path) -> Result<Manager, HostError> {
        let host = Host::load(locale_dir)?;
        let quest_dir = locale_dir.join("quest");
        let path = quest_dir.join("questnpc.txt");
        let questnpc = fs::read(&path).map_err(|error| HostError::Io(path, error))?;
        let path = quest_dir.join("questcategory.txt");
        let questcategory = fs::read(&path).unwrap_or_else(|error| {
            tracing::error!(path = %path.display(), %error, "QUEST Cannot open 'questcategory.txt'");
            Vec::new()
        });
        Ok(Manager::new(host, &questnpc, &questcategory))
    }

    /// Indexes the scripts of `host` by NPC, with the NPC names of `questnpc` and the quest
    /// categories of `questcategory`.
    pub fn new(host: Host, questnpc: &[u8], questcategory: &[u8]) -> Manager {
        let mut files: BTreeMap<u32, NpcFiles> = BTreeMap::new();
        let mut npc_ids = BTreeMap::new();
        for (vnum, name) in parse_questnpc(questnpc) {
            files.entry(vnum).or_default().read(&host, &name);
            npc_ids.insert(name, vnum);
        }
        files.entry(0).or_default().read(&host, b"notarget");
        for vnum in numeric_dirs(&host) {
            files
                .entry(vnum)
                .or_default()
                .read(&host, vnum.to_string().as_bytes());
        }
        let npcs = files
            .into_iter()
            .map(|(vnum, files)| (vnum, files.build(&host)))
            .collect();
        let event_flags = [
            (b"guild_withdraw_delay".to_vec(), 1),
            (b"guild_disband_delay".to_vec(), 1),
        ]
        .into_iter()
        .collect();
        let categories = parse_questcategory(questcategory, host.quest_indices());
        Manager {
            host,
            npcs,
            npc_ids,
            event_flags,
            pcs: BTreeMap::new(),
            quest_npc_vid: 0,
            categories,
            timer_ids: BTreeMap::new(),
            timer_arg: 0,
            selected: None,
        }
    }

    /// The Lua state and the loaded scripts.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// `GetQuestStateName` (`G/questmanager.cpp:1154-1169`): the name of state
    /// `state` of quest `quest`, or the empty name for a state the quest does
    /// not have. What `SendFlagList` prints for a `.__status` flag.
    pub fn state_name(&self, quest: &[u8], state: i32) -> Vec<u8> {
        self.host
            .quests()
            .iter()
            .find(|candidate| candidate.name == quest)
            .and_then(|quest| quest.states.iter().find(|(_, number)| **number == state))
            .map(|(name, _)| name.clone())
            .unwrap_or_default()
    }

    /// `GetQuestStateIndex` (`G/questmanager.cpp:1172-1190`): the number of
    /// state `state` of quest `quest`, 0 for a state the quest does not have.
    /// What `do_set_state` writes.
    pub fn state_index(&self, quest: &[u8], state: &[u8]) -> i32 {
        self.host
            .quests()
            .iter()
            .find(|candidate| candidate.name == quest)
            .and_then(|quest| quest.states.get(state))
            .copied()
            .unwrap_or(0)
    }

    /// `do_set_state`'s write (`G/cmd_gm.cpp:2915-2948`): the status flag of
    /// quest `quest` of character `pid` becomes `value`, and the quest list
    /// follows when the quest is listed. Creates the record, as `GetPCForce`
    /// does. No timer is cleared: the port holds no `m_TimerMap` (the timer
    /// names are still `missing`).
    pub fn set_quest_state(&mut self, pid: u32, quest: &[u8], value: i32) {
        let status = [quest, b".__status"].concat();
        self.set_quest_flag(pid, &status, value);
        let Some(index) = self.host.quest_index(quest) else {
            return;
        };
        if let Some(pc) = self.pcs.get_mut(&pid) {
            if pc.quests.contains_key(&index) {
                pc.quests.insert(index, value);
            }
        }
    }

    /// The name of the quest character `pid` ran last (`GetCurrentQuestName`,
    /// `G/questpc.cpp:42-45`): `m_stCurQuest`, empty when it ran none. What
    /// `do_qf` names.
    pub fn current_quest_name(&self, pid: u32) -> Vec<u8> {
        self.pcs
            .get(&pid)
            .map_or_else(Vec::new, |pc| pc.current.clone())
    }

    /// `do_clear_quest`'s wipe (`G/questpc.cpp:681-701`): every flag of
    /// character `pid` whose name starts with `quest.` is set to 0, and the
    /// listed quest rests at 0. A flag already at 0 is no flag — legacy's
    /// `SetFlag(name, 0)` deletes it — so the wipe deletes in one pass.
    /// Creates the record, as `GetPCForce` does. No timer is cleared: the
    /// port holds no `m_TimerMap` (the timer names are still `missing`).
    pub fn clear_quest(&mut self, pid: u32, quest: &[u8]) {
        self.pcs.entry(pid).or_default();
        let prefix = [quest, b"."].concat();
        if let Some(pc) = self.pcs.get_mut(&pid) {
            pc.flags.retain(|name, _| !name.starts_with(&prefix));
        }
        let Some(index) = self.host.quest_index(quest) else {
            return;
        };
        if let Some(pc) = self.pcs.get_mut(&pid) {
            if pc.quests.contains_key(&index) {
                pc.quests.insert(index, 0);
            }
        }
    }

    /// Whether NPC race `race` has any script.
    pub fn has_npc(&self, race: u32) -> bool {
        self.npcs.get(&race).is_some_and(|npc| !npc.is_empty())
    }
    /// Whether the manager holds a PC record for `pid` — it has seen the character
    /// (a click) or force-created it (a `party_kill` for its leader).
    pub fn has_seen(&self, pid: u32) -> bool {
        self.pcs.contains_key(&pid)
    }

    /// Whether a script of character `pid` is suspended, waiting for the client.
    pub fn is_running(&self, pid: u32) -> bool {
        self.pcs.get(&pid).is_some_and(|pc| pc.running.is_some())
    }

    /// The quest NPC of character `pid` (`m_dwQuestNPCVID`, `G/char.cpp:7615-7618`): the
    /// NPC the last click named, 0 with none.
    pub fn npc_vid(&self, pid: u32) -> u32 {
        self.pcs.get(&pid).map_or(0, |pc| pc.npc_vid)
    }

    /// A quest flag of character `pid`, such as `QUEST.__status`, the state a quest last ran in.
    pub fn quest_flag(&self, pid: u32, name: &[u8]) -> Option<i32> {
        self.pcs.get(&pid)?.flags.get(name).copied()
    }
    /// `quest::PC::SetFlag` (`G/questpc.cpp:52-76`): sets a quest flag of character
    /// `pid`, which the book training counts read (`G/char_skill.cpp:407,592,600`).
    /// Creates the character's record, as legacy's `GetPCForce` does for a writer.
    ///
    /// A flag whose value is zero is no flag at all: the legacy deletes it
    /// (`quest::PC::DeleteFlag`, `G/questpc.cpp:78-90`), so a flag that was cleared
    /// reads as absent and a save does not write it. A flag that is not there is not
    /// deleted, and no record is made for it.
    pub fn set_quest_flag(&mut self, pid: u32, name: &[u8], value: i32) {
        if value == 0 {
            if let Some(pc) = self.pcs.get_mut(&pid) {
                pc.flags.remove(name);
            }
            return;
        }
        self.pcs
            .entry(pid)
            .or_default()
            .flags
            .insert(name.to_vec(), value);
    }

    /// `quest::PC::DeleteFlag` (`G/questpc.cpp:78-90`): deletes a quest flag of
    /// character `pid`. Answers whether the flag was there, which is what
    /// `do_delqf` chats. A missing record holds no flag.
    pub fn delete_quest_flag(&mut self, pid: u32, name: &[u8]) -> bool {
        self.pcs
            .get_mut(&pid)
            .is_some_and(|pc| pc.flags.remove(name).is_some())
    }

    /// `CInputDB::QuestLoad` (`G/input_db.cpp:1079-1101`) with `quest::PC::SetFlag`
    /// (`G/questpc.cpp:55-72`): the stored flags of character `pid`.
    ///
    /// The load sets the flags on a record that holds none, so they replace what the
    /// record had: the character's flags are the stored ones.
    ///
    /// The quest list follows the flags (`quest::PC::Build`, `G/questpc.cpp:664-679`): a
    /// flag whose name ends in `.__status` is the state of the quest the rest of the name
    /// names, and the quest the script runs is that one. A flag whose value is zero is no
    /// flag and is dropped, as legacy's `SetFlag` deletes it.
    pub fn load_flags(&mut self, pid: u32, flags: impl IntoIterator<Item = (Vec<u8>, i32)>) {
        let pc = self.pcs.entry(pid).or_default();
        pc.flags.clear();
        for (name, value) in flags {
            if value == 0 {
                pc.flags.remove(&name);
            } else {
                pc.flags.insert(name, value);
            }
        }
        pc.quests.clear();
        for (name, value) in &pc.flags {
            let Some(quest_name) = name.as_slice().strip_suffix(b".__status") else {
                continue;
            };
            let Some(index) = self.host.quest_index(quest_name) else {
                continue;
            };
            pc.quests.insert(index, *value);
        }
    }

    /// `quest::PC::Save` (`G/questpc.cpp:519-590`): the flags of character `pid` that a
    /// save writes, in name order.
    ///
    /// A flag whose value is zero is no flag and is not written: the legacy deletes it
    /// instead (`quest::PC::SetFlag`, `G/questpc.cpp:60-64`).
    pub fn flags_of(&self, pid: u32) -> Vec<(Vec<u8>, i32)> {
        let Some(pc) = self.pcs.get(&pid) else {
            return Vec::new();
        };
        pc.flags
            .iter()
            .filter(|(_, value)| **value != 0)
            .map(|(name, value)| (name.clone(), *value))
            .collect()
    }

    /// `game.get_event_flag`: an event flag, 0 when it was never set.
    pub fn event_flag(&self, name: &[u8]) -> i32 {
        self.event_flags.get(name).copied().unwrap_or(0)
    }

    /// `game.set_event_flag` and `do_event_flag`'s set: the flag `name` takes `value`.
    pub fn set_event_flag(&mut self, name: &[u8], value: i32) {
        self.event_flags.insert(name.to_vec(), value);
    }

    /// Every event flag, in name order, for `do_get_event_flag`'s list.
    pub fn event_flags(&self) -> impl Iterator<Item = (&Vec<u8>, &i32)> {
        self.event_flags.iter()
    }

    /// `CQuestManager::Click`: character `pid` clicked the NPC `npc_vid` of race `race`. Returns
    /// whether a quest took the click, in which case the NPC's shop does not open.
    ///
    /// A click on a character one of the clicker's quests marked asks that quest first
    /// (`G/questmanager.cpp:934-945`): its `click` script for the mark at the character's
    /// state of it (`Run::on_target`) takes the click, and the NPC's own scripts do not run.
    pub fn click(&mut self, pid: u32, npc_vid: u32, race: u32, game: &mut dyn Game) -> bool {
        // `CHARACTER::OnClick` records the NPC before it asks the quests (`SetQuestNPCID`).
        self.pcs.entry(pid).or_default().npc_vid = npc_vid;
        self.operate(pid, game, |run, pc, npcs| {
            let marked = run.cx.borrow().game.target_at(npc_vid);
            if let Some((quest, name)) = marked {
                if run.on_target(pc, quest, &name, b"click") {
                    return true;
                }
            }
            let Some(npc) = npcs.get(&race) else {
                tracing::debug!(pid, race, "QUEST click: no quest NPC of this race");
                return false;
            };
            if npc.has_chat() && run.on_chat(pc, npc) {
                return true;
            }
            run.handle_event(pc, npc, "click")
        })
    }
    /// `CQuestManager::UseItem` (`G/questmanager.cpp:819-860`): character `pid` used the
    /// item of ID `item` and vnum `vnum`, an `ITEM_QUEST` with neither quest-use flag, so it
    /// becomes the quest item (`SetCurrentItem`) and the item's own `use` scripts run
    /// (`NPC::OnUseItem`, `G/questnpc.cpp:283-289`, then `HandleEvent`). Returns whether one
    /// ran. Legacy's `GetPC` makes the record of a character it never saw, as a click does.
    pub fn use_item(&mut self, pid: u32, (item, vnum): (u32, u32), game: &mut dyn Game) -> bool {
        self.pcs.entry(pid).or_default().item = item;
        self.operate(pid, game, |run, pc, npcs| match npcs.get(&vnum) {
            Some(npc) => run.handle_event(pc, npc, "use"),
            None => false,
        })
    }

    /// `CQuestManager::SIGUse` (`G/questmanager.cpp:863-886`): character `pid` used the
    /// item of ID `item` whose `SIGVnum` is `sig`, so it becomes the quest item
    /// (`SetCurrentItem`) and the group's `sig_use` scripts run (`NPC::OnSIGUse`,
    /// `G/questnpc.cpp:291-297`, then `HandleEvent`). Returns whether one ran. Legacy's
    /// `GetPC` makes the record of a character it never saw, as a click does, and the
    /// flags ride the login, so no `Loading quest` wait exists.
    pub fn sig_use(&mut self, pid: u32, (item, sig): (u32, u32), game: &mut dyn Game) -> bool {
        self.pcs.entry(pid).or_default().item = item;
        self.operate(pid, game, |run, pc, npcs| match npcs.get(&sig) {
            Some(npc) => run.handle_event(pc, npc, "sig_use"),
            None => false,
        })
    }

    /// `CQuestManager::TakeItem` (`G/questmanager.cpp:791-815`): character `pid` gave the item
    /// of ID `item` to the NPC of race `race`, so it becomes the quest item (`SetCurrentItem`)
    /// and the NPC's `take` scripts run (`NPC::OnTakeItem`, `G/questnpc.cpp:278-281`, then
    /// `HandleEvent`). Returns whether one ran. The caller records the NPC first
    /// (`SetQuestNPCID`), as `CHARACTER::ReceiveItem` does. Legacy's `GetPC` makes the
    /// record of a character it never saw, as a click does, and the flags ride the login,
    /// so no `Loading quest` wait exists.
    pub fn take(&mut self, pid: u32, (item, race): (u32, u32), game: &mut dyn Game) -> bool {
        self.pcs.entry(pid).or_default().item = item;
        self.operate(pid, game, |run, pc, npcs| match npcs.get(&race) {
            Some(npc) => run.handle_event(pc, npc, "take"),
            None => false,
        })
    }

    /// `CQuestManager::LevelUp`: character `pid` leveled up, so the no-target quests' `levelup`
    /// scripts run (`questmanager.cpp:667-681`) the way `NPC::OnLevelUp` runs them
    /// (`HandleReceiveAllEvent`, `G/questnpc.cpp:348-351`). Every owner script with a `levelup`
    /// trigger is such a script, and neither takes a chat menu or an argument. A character the
    /// manager never saw still misses, as legacy's `GetPC` returns NULL for it (`sys_err` there).
    pub fn levelup(&mut self, pid: u32, game: &mut dyn Game) -> bool {
        if !self.pcs.contains_key(&pid) {
            tracing::warn!("QUEST LEVELUP_EVENT no such pc id : {pid}");
            return false;
        }
        self.operate(pid, game, |run, pc, npcs| match npcs.get(&QUEST_NO_NPC) {
            Some(npc) => run.handle_receive_all(pc, npc, "levelup", Wait::Kept),
            None => false,
        })
    }

    /// `CQuestManager::Target` (`G/questmanager.cpp:723-736`): the `verb` (`arrive`, `die`) of
    /// character `pid`'s target of quest `quest` named `name`, which `target_event` reports,
    /// runs that quest's script for it (`Run::on_target`). Returns whether one ran or a
    /// waiting script kept it from running. A character the manager never saw misses, as
    /// legacy's `GetPC` returns NULL for it.
    pub fn target(
        &mut self,
        pid: u32,
        (quest, name): (u32, &[u8]),
        verb: &[u8],
        game: &mut dyn Game,
    ) -> bool {
        if !self.pcs.contains_key(&pid) {
            return false;
        }
        self.operate(pid, game, |run, pc, _| run.on_target(pc, quest, name, verb))
    }

    /// `CQuestManager::Login` (`G/questmanager.cpp:541-556`) with `NPC::OnLogin`
    /// (`G/questnpc.cpp:353-397`): character `pid` entered the game with its flags loaded,
    /// so every no-target `login` script runs at its quest's state, and a script that
    /// still waits for its client when it returns is closed at once
    /// (`HandleReceiveAllNoWaitEvent`); then every `letter` script runs the same way, but
    /// left waiting (`HandleReceiveAllEvent`). Returns whether a `login` script ran. A
    /// character the manager never saw misses, as legacy's `GetPC` returns NULL for it.
    pub fn login(&mut self, pid: u32, game: &mut dyn Game) -> bool {
        if !self.pcs.contains_key(&pid) {
            tracing::warn!("QUEST no such pc id : {pid}");
            return false;
        }
        self.operate(pid, game, |run, pc, npcs| {
            let Some(npc) = npcs.get(&QUEST_NO_NPC) else {
                return false;
            };
            let ran = run.handle_receive_all(pc, npc, "login", Wait::Closed);
            run.handle_receive_all(pc, npc, "letter", Wait::Kept);
            ran
        })
    }

    /// `CQuestManager::ServerTimer` (`G/questmanager.cpp:635-643`): the timer `name`
    /// fires with `arg`, so the timer NPC's `server_timer` scripts run for PID 0, which
    /// `GetPCForce` makes when unseen. The dungeon selection clears and `arg` becomes
    /// what `get_server_timer_arg` pushes. Returns whether a script ran; a suspended
    /// PC 0 keeps the pass from running, as `HandleReceiveAllEvent`'s `IsRunning`
    /// check does.
    pub fn server_timer(&mut self, name: &[u8], arg: u32, game: &mut dyn Game) -> bool {
        let id = self.load_timer_script(name);
        self.timer_arg = arg;
        self.selected = None;
        self.operate(0, game, |run, pc, npcs| match npcs.get(&id) {
            Some(npc) => run.handle_receive_all(pc, npc, "server_timer", Wait::Kept),
            None => false,
        })
    }

    /// `CQuestManager::LoadTimerScript` (`G/questmanager.cpp:1202-1218`): the NPC id
    /// of the timer `name`, counted down from `u32::MAX`, indexing the timer's
    /// scripts under `object/NAME` on first use.
    fn load_timer_script(&mut self, name: &[u8]) -> u32 {
        if let Some(id) = self.timer_ids.get(name) {
            return *id;
        }
        let id = u32::MAX - u32::try_from(self.timer_ids.len()).unwrap_or(u32::MAX);
        self.timer_ids.insert(name.to_vec(), id);
        let mut files = NpcFiles::default();
        files.read(&self.host, name);
        self.npcs.insert(id, files.build(&self.host));
        id
    }

    /// `CQuestManager::GetServerTimerArg` (`G/questmanager.cpp:2014-2017`): the firing
    /// timer's argument, 0 outside one.
    pub fn server_timer_arg(&self) -> u32 {
        self.timer_arg
    }

    /// `CQuestManager::SelectDungeon` (`G/questmanager.cpp:2019-2022`): the `d.*`
    /// names read the dungeon under `index` while no character runs; `None` with
    /// none. What `d.select` writes.
    pub fn select_dungeon(&mut self, index: Option<u32>) {
        self.selected = index;
    }

    /// The selected dungeon `d.select` named, which the `d.*` names read while no
    /// character runs.
    pub fn selected_dungeon(&self) -> Option<u32> {
        self.selected
    }

    /// `CHARACTER::SetQuestNPCID` (`G/char.cpp:7610-7613`): records `victim` as the quest NPC
    /// for the next dispatch. `Reward` calls it before `Kill`; `operate` publishes the
    /// stored VID to the bridge (`cx.npc_vid`).
    pub fn set_quest_npc(&mut self, victim: u32) {
        self.quest_npc_vid = victim;
    }
    /// `CQuestManager::Kill`: character `pid` killed race `race` (`questmanager.cpp:574-611`).
    /// A race at or past `MAIN_RACE_MAX_NUM` (8, `G/char.h:62-73`) runs its own `kill`
    /// scripts, and the no-target scripts' `kill` always runs; `ENABLE_PARTYKILL` (defined,
    /// `:573`) then runs `party_kill` on both for `leader`'s PC (`GetPCForce` — created
    /// when unseen). `leader` is `Some` killer's own PID when partyless and `None` when the
    /// killer's leader is offline (`:598-600` — a NULL leader runs nothing). A killer the
    /// manager never saw still misses the whole call, as legacy's `GetPC` returns NULL for
    /// it (`sys_err` there).
    pub fn kill(&mut self, pid: u32, race: u32, leader: Option<u32>, game: &mut dyn Game) -> bool {
        if !self.pcs.contains_key(&pid) {
            tracing::warn!("QUEST KILL_EVENT no such pc id : {pid}");
            return false;
        }
        // `SetQuestNPCID` ran on the caller (`Reward`, `G/char_battle.cpp:810`).
        let mut ran = self.operate(pid, game, |run, pc, npcs| {
            let mut ran = false;
            if race >= MAIN_RACE_MAX_NUM {
                if let Some(npc) = npcs.get(&race) {
                    ran |= run.handle_event(pc, npc, "kill");
                }
            }
            if let Some(npc) = npcs.get(&QUEST_NO_NPC) {
                ran |= run.handle_event(pc, npc, "kill");
            }
            ran
        });
        let Some(leader) = leader else {
            return ran;
        };
        // The victim publishes to the leader's half too (`npc.getrace` reads it there):
        // the killer's `operate` consumed the stored VID, so it is set again.
        self.quest_npc_vid = self.pcs.get(&pid).map_or(0, |pc| pc.npc_vid);
        // `GetPCForce(leader)`: `operate`'s entry creates the unseen PC.
        ran |= self.operate(leader, game, |run, pc, npcs| {
            let mut ran = false;
            if race >= MAIN_RACE_MAX_NUM {
                if let Some(npc) = npcs.get(&race) {
                    ran |= run.handle_event(pc, npc, "party_kill");
                }
            }
            if let Some(npc) = npcs.get(&QUEST_NO_NPC) {
                ran |= run.handle_event(pc, npc, "party_kill");
            }
            ran
        });
        ran
    }

    /// `CInputMain::ScriptAnswer`: an answer above 250 continues a `wait`, any other picks entry
    /// `answer` of a menu.
    pub fn answer(&mut self, pid: u32, answer: u8, game: &mut dyn Game) {
        if answer > ANSWER_RESUME {
            self.resume(pid, game);
        } else {
            self.select(pid, u32::from(answer), game);
        }
    }

    /// `CQuestManager::Letter` (`G/questmanager.cpp:502-514`): the named quest's letter
    /// event at the given state.
    ///
    /// The letter is how the war's members are asked to join the war map: legacy's
    /// `CGuild::GuildWarEntryAsk` sends the `guild_war_join` letter to every online
    /// member of the two guilds (`G/guild_war.cpp:680-727`), and the quest's button
    /// calls `guild.war_enter`.
    pub fn letter(&mut self, pid: u32, quest: u32, state: i32, game: &mut dyn Game) -> bool {
        self.operate(pid, game, |run, pc, npcs| {
            let Some(npc) = npcs.get(&QUEST_NO_NPC) else {
                return false;
            };
            run.event_script(pc, npc, "letter", quest, state)
        })
    }

    /// `CQuestManager::QuestButton` (`G/questmanager.cpp:767-789`) and `NPC::OnButton`
    /// (`G/questnpc.cpp:810-859`): the named quest's button event, at the character's
    /// state of that quest or at its start.
    ///
    /// The client's quest button carries the quest's number (`makequestbutton`,
    /// `quest/questlib.lua:117-122`), and the high bit asks for the quest's info
    /// instead of its button (`CInputMain::ScriptButton`, `G/input_main.cpp:2179-2198`).
    pub fn button(&mut self, pid: u32, quest: u32, game: &mut dyn Game) -> bool {
        // `CInputMain::ScriptButton` (`G/input_main.cpp:2184-2188`): a button pressed while
        // the character waits on a confirm is not a button at all; the wait hears
        // `CONFIRM_TIMEOUT` instead.
        if self
            .pcs
            .get(&pid)
            .is_some_and(|pc| pc.confirm_wait.is_some())
        {
            self.confirm(pid, CONFIRM_TIMEOUT, None, game);
            return false;
        }
        self.operate(pid, game, |run, pc, npcs| {
            // A running script holds the character (`NPC::OnButton` refuses one).
            if pc.running.is_some() {
                return false;
            }
            let Some(npc) = npcs.get(&QUEST_NO_NPC) else {
                return false;
            };
            let state = if let Some(state) = pc.quests.get(&quest) {
                *state
            } else {
                if !run.can_start(quest) {
                    return false;
                }
                0
            };
            run.event_script(pc, npc, "button", quest, state)
        })
    }

    /// `CQuestManager::RequestEventQuest` (`G/questmanager.cpp:1956-1963`): the client's
    /// event panel names a quest, whose button runs when no script waits. An unknown
    /// name answers nothing: `GetQuestIndexByName` answers 0, which no quest has.
    pub fn event_quest(&mut self, pid: u32, name: &[u8], game: &mut dyn Game) -> bool {
        let Some(quest) = self.host.quest_indices().get(name).copied() else {
            return false;
        };
        self.button(pid, quest, game)
    }

    /// `CQuestManager::QuestInfo` (`G/questmanager.cpp:739-765`) and `NPC::OnInfo`
    /// (`G/questnpc.cpp:761-808`): the client's quest window asked about quest `quest`
    /// (`CG_SCRIPT_BUTTON` with the high bit, `CInputMain::ScriptButton`,
    /// `G/input_main.cpp:2188-2192`), so its `info` script runs at the character's state of
    /// it. Unlike the button, a quest the character has no state of runs nothing.
    pub fn info(&mut self, pid: u32, quest: u32, game: &mut dyn Game) -> bool {
        self.operate(pid, game, |run, pc, npcs| {
            if pc.running.is_some() {
                return false;
            }
            let Some(state) = pc.quests.get(&quest).copied() else {
                tracing::warn!("QUEST no quest by (quest {quest})");
                return false;
            };
            let Some(npc) = npcs.get(&QUEST_NO_NPC) else {
                return false;
            };
            if run.event_script(pc, npc, "info", quest, state) {
                return true;
            }
            tracing::warn!(
                "QUEST no info script by state {state} (quest {})",
                String::from_utf8_lossy(&run.quest_name(quest))
            );
            false
        })
    }

    /// `CQuestManager::SelectItem` (`G/questmanager.cpp:179-194`): the selection the
    /// character picked while its script waited in `select_item`, or 0 for a window the
    /// client closed. A script that waits nowhere, or waits elsewhere, is undisturbed.
    pub fn select_item(&mut self, pid: u32, selection: u32, game: &mut dyn Game) {
        if !self.suspended(pid, Suspend::SelectItem) {
            tracing::warn!("wrong QUEST_SELECT_ITEM request! : {pid}");
            return;
        }
        self.operate(pid, game, |run, pc, _| {
            pc.send_done = true;
            let answer = Value::Number(f64::from(selection));
            run.resume(pc, Ok(MultiValue::from_iter([answer])));
        });
    }

    /// `CQuestManager::Confirm` (`G/questmanager.cpp:196-235`): `confirm`'s answer for the
    /// character `waiter` whose script waits for it, from the character `replier` whose
    /// client sent it (`None` is the timeout's own voice, whose `pc2` legacy leaves 0).
    /// Returns whether the script heard it.
    ///
    /// A script that waits nowhere, or waits elsewhere, is undisturbed. An answer from a
    /// character the script does not wait for is refused with a log line, and the wait
    /// stands (`pc2 && !pPC->IsConfirmWait(pc2)`); the timeout has no `pc2` and always
    /// passes. A heard answer first closes the waiting window (`[END_CONFIRM_WAIT]` in
    /// `QUEST_SKIN_NOWINDOW`), then the script resumes with it.
    pub fn confirm(
        &mut self,
        waiter: u32,
        confirm: u8,
        replier: Option<u32>,
        game: &mut dyn Game,
    ) -> bool {
        if !self.suspended(waiter, Suspend::Confirm) {
            tracing::warn!("wrong QUEST_CONFIRM request! : {waiter}");
            return false;
        }
        if replier.is_some_and(|replier| {
            self.pcs
                .get(&waiter)
                .and_then(|pc| pc.confirm_wait)
                .is_none_or(|wait| wait != replier)
        }) {
            tracing::warn!("wrong QUEST_CONFIRM replier! : {waiter}");
            return false;
        }
        self.operate(waiter, game, |run, pc, _| {
            pc.confirm_wait = None;
            pc.send_done = true;
            run.end_confirm_wait(pc);
            let answer = Value::Number(f64::from(confirm));
            run.resume(pc, Ok(MultiValue::from_iter([answer])));
        });
        true
    }

    /// `CQuestManager::Input`: the text a script's `input()` waits for.
    pub fn input(&mut self, pid: u32, msg: &[u8], game: &mut dyn Game) {
        let suspend = self
            .pcs
            .get(&pid)
            .map(|pc| pc.running.as_ref().map(|running| running.suspend));
        match suspend {
            None => tracing::warn!("no pc! : {pid}"),
            Some(None) => {
                tracing::warn!("no quest running for pc, cannot process input : {pid}");
            }
            Some(Some(Suspend::Input)) => {
                let end = msg.iter().position(|byte| *byte == 0).unwrap_or(msg.len());
                let msg = &msg[..end];
                self.operate(pid, game, |run, pc, _| {
                    pc.send_done = true;
                    let args = run
                        .lua()
                        .create_string(msg)
                        .map(|text| MultiValue::from_iter([Value::String(text)]));
                    run.resume(pc, args);
                });
            }
            Some(Some(suspend)) => {
                tracing::warn!("not wait for a input : {pid} {suspend:?}");
            }
        }
    }

    /// `CQuestManager::LogoutPC` (`G/questmanager.cpp:516-535`): the running script ends
    /// unfinished and the no-target `logout` scripts run (`NPC::OnLogout`,
    /// `G/questnpc.cpp:399-402`). The record stays for the save to take.
    pub fn logout(&mut self, pid: u32, game: &mut dyn Game) {
        if !self.pcs.contains_key(&pid) {
            return;
        }
        self.operate(pid, game, |run, pc, npcs| {
            let Some(npc) = npcs.get(&QUEST_NO_NPC) else {
                return;
            };
            run.handle_receive_all(pc, npc, "logout", Wait::Kept);
        });
    }

    /// `CQuestManager::DisconnectPC` (`G/questmanager.cpp:1068-1071`): the record goes,
    /// after the save took its flags.
    pub fn disconnect(&mut self, pid: u32) {
        self.pcs.remove(&pid);
    }

    /// `CQuestManager::Select`.
    fn select(&mut self, pid: u32, selection: u32, game: &mut dyn Game) {
        if !self.suspended(pid, Suspend::Select) {
            tracing::warn!("wrong QUEST_SELECT request! : {pid}");
            return;
        }
        self.operate(pid, game, |run, pc, _| {
            pc.send_done = true;
            let Some(running) = pc.running.take() else {
                return;
            };
            if running.chat.is_empty() {
                pc.running = Some(running);
                let answer = Value::Number(f64::from(selection) + 1.0);
                run.resume(pc, Ok(MultiValue::from_iter([answer])));
                return;
            }
            // A chat menu: the entry runs as its own quest, and the menu's thread is closed.
            if let Some(entry) = usize::try_from(selection)
                .ok()
                .and_then(|index| running.chat.get(index))
            {
                let name = run.quest_name(entry.quest);
                let code = entry.script.clone();
                run.execute(pc, entry.quest, name, entry.state, code, Vec::new());
            } else {
                pc.send_done = true;
                run.goto_end(pc);
            }
        });
    }

    /// `CQuestManager::Resume`.
    fn resume(&mut self, pid: u32, game: &mut dyn Game) {
        if !self.suspended(pid, Suspend::Pause) {
            tracing::warn!("wrong QUEST_WAIT request! : {pid}");
            return;
        }
        self.operate(pid, game, |run, pc, _| {
            pc.send_done = true;
            run.resume(pc, Ok(MultiValue::new()));
        });
    }

    fn suspended(&self, pid: u32, suspend: Suspend) -> bool {
        self.pcs
            .get(&pid)
            .and_then(|pc| pc.running.as_ref())
            .is_some_and(|running| running.suspend == suspend)
    }

    /// Runs `op` for character `pid` with the bridge set for its whole length.
    /// `pc` reaches the bridge (`Cx::pc`) so a name whose answer mutates the
    /// character's quest state (`pc.learn_grand_master_skill`'s count) does it
    /// now, while the script that asked still waits for the answer.
    fn operate<R: Default>(
        &mut self,
        pid: u32,
        game: &mut dyn Game,
        op: impl FnOnce(&Run<'_, '_>, &mut Pc, &BTreeMap<u32, Npc>) -> R,
    ) -> R {
        let Manager {
            host,
            npcs,
            npc_ids,
            event_flags,
            pcs,
            quest_npc_vid,
            categories,
            timer_ids: _,
            timer_arg,
            selected,
        } = self;
        let (npc_vid, current, item) = {
            let pc = pcs.entry(pid).or_default();
            // `SetQuestNPCID` overwrites what the click recorded (`Reward` runs after any
            // click, and the kill's scripts read the victim).
            if *quest_npc_vid != 0 {
                pc.npc_vid = *quest_npc_vid;
                *quest_npc_vid = 0;
            }
            (pc.npc_vid, pc.current.clone(), pc.item)
        };
        // The bridge owns every PC's flags while Lua runs (`Cx::borrowed`): a name
        // whose answer mutates quest state does it now, while the script that asked
        // still waits, and a `q.begin_other_pc_block` switches the live map to the
        // blocked PC's. Swapped out and back — `execute` also writes `__status`
        // into the empty `pc.flags`, which is merged back on top, but never while
        // a bridge call holds the `RefCell`.
        let mut borrowed: BTreeMap<u32, BTreeMap<Vec<u8>, i32>> = pcs
            .iter_mut()
            .map(|(pid, pc)| (*pid, std::mem::take(&mut pc.flags)))
            .collect();
        let host: &Host = host;
        let npcs: &BTreeMap<u32, Npc> = npcs;
        let quest_indices = host.quest_indices();
        let cx = Cx {
            output: Output::default(),
            game,
            event_flags,
            npc_ids,
            quest_indices,
            npc_vid,
            timer_arg: *timer_arg,
            selected: *selected,
            quest: current,
            pending_state: None,
            pending_others: Vec::new(),
            borrowed: Some(&mut borrowed),
            current: pid,
            stack: Vec::new(),
            window: Window::default(),
            categories,
            item,
            confirm_wait: None,
        };
        let cell = RefCell::new(cx);
        let result = {
            let pc = pcs.entry(pid).or_default();
            let run = Run {
                host,
                cx: &cell,
                no_npc: npcs.get(&QUEST_NO_NPC),
                changes: Cell::new(0),
            };
            run.bridged(|| op(&run, pc, npcs))
        };
        let cx = cell.into_inner();
        let pc = pcs.entry(pid).or_default();
        pc.item = cx.item;
        *selected = cx.selected;
        let unsent = cx.output.script.clone();
        // The borrowed maps are the live ones: the bridge's writes landed there, and
        // every `pc.flags` stood empty meanwhile — except `execute`'s `__status`,
        // which wrote the root's empty map and is merged back on top.
        for (name, value) in std::mem::take(&mut pc.flags) {
            borrowed.entry(pid).or_default().insert(name, value);
        }
        for (pid, pc) in pcs.iter_mut() {
            if let Some(map) = borrowed.remove(pid) {
                pc.flags = map;
            }
        }
        for (pid, map) in borrowed {
            pcs.entry(pid).or_default().flags = map;
        }
        if !unsent.is_empty() {
            tracing::warn!(
                pid,
                text = %String::from_utf8_lossy(&unsent),
                "QUEST dialog text no script step sent is dropped"
            );
        }
        result
    }
}

/// What an NPC holds by event, quest number and state.
type ByState<T> = BTreeMap<&'static str, BTreeMap<u32, BTreeMap<i32, T>>>;

/// The scripts of one NPC (`quest::NPC`).
#[derive(Debug, Default)]
struct Npc {
    /// `m_mapOwnQuest`: the script that runs.
    own: ByState<Code>,
    /// `m_mapOwnArgQuest`: the menu entries.
    own_arg: ByState<Vec<ArgScript>>,
}

impl Npc {
    /// `NPC::HasChat`.
    fn has_chat(&self) -> bool {
        self.own_arg
            .get("chat")
            .is_some_and(|quests| !quests.is_empty())
    }

    fn is_empty(&self) -> bool {
        self.own.is_empty() && self.own_arg.is_empty()
    }
}

/// A menu entry (`AArgScript`): `when EVENT.ARG with CONDITION begin SCRIPT end`.
#[derive(Clone, Debug)]
struct ArgScript {
    /// The condition, `None` when there is none.
    when: Option<Code>,
    /// The `object/` file of the argument, which `ScriptToString` evaluates for the label.
    arg: Option<Vec<u8>>,
    script: Code,
    /// The quest and state the entry's script runs under: those of its `.script` file, else 0.
    quest: u32,
    state: i32,
}

/// The `object/` files of one NPC, before they are compiled into an [`Npc`].
#[derive(Debug, Default)]
struct NpcFiles {
    own: ByState<Vec<Vec<u8>>>,
    own_arg: ByState<Vec<ArgFiles>>,
}

#[derive(Clone, Debug, Default)]
struct ArgFiles {
    when: Vec<Vec<u8>>,
    arg: Option<Vec<u8>>,
    script: Vec<Vec<u8>>,
    owner: Option<(u32, i32)>,
}

/// What a file name below an event directory says (`NPC::LoadStateScript`).
#[derive(Debug, PartialEq, Eq)]
enum FileName<'a> {
    /// `QUEST.STATE`: a script that runs on the event.
    Own { quest: &'a [u8], state: &'a [u8] },
    /// `QUEST.STATE.INDEX.TYPE`: part of a menu entry.
    Arg {
        quest: &'a [u8],
        state: &'a [u8],
        index: i64,
        kind: &'a [u8],
    },
    /// `QUEST.STATE.INDEX` with no type.
    NoIndex,
    /// `QUEST.STATE.INDEX.TYPE.MORE`.
    BadName,
}

fn parse_file_name(name: &[u8]) -> FileName<'_> {
    let Some(first) = name.iter().position(|byte| *byte == b'.') else {
        // Legacy's second `find` starts over at 0, so the state name is the whole name too.
        return FileName::Own {
            quest: name,
            state: name,
        };
    };
    let quest = &name[..first];
    let after = &name[first + 1..];
    let Some(second) = after.iter().position(|byte| *byte == b'.') else {
        return FileName::Own {
            quest,
            state: after,
        };
    };
    let state = &after[..second];
    let rest = &after[second + 1..];
    let Some(third) = rest.iter().position(|byte| *byte == b'.') else {
        return FileName::NoIndex;
    };
    let kind = &rest[third + 1..];
    if kind.contains(&b'.') {
        return FileName::BadName;
    }
    FileName::Arg {
        quest,
        state,
        index: strtol(&rest[..third]),
        kind,
    }
}

impl NpcFiles {
    /// `NPC::Set`: the files of every event directory below `object/DIR`.
    fn read(&mut self, host: &Host, dir: &[u8]) {
        for event in EVENTS {
            let prefix = [dir, b"/", event.as_bytes(), b"/"].concat();
            let files = host
                .objects()
                .range(prefix.clone()..)
                .map(|(file, _)| file)
                .take_while(|file| file.starts_with(&prefix));
            for file in files {
                let name = &file[prefix.len()..];
                // `qc` writes no deeper file; legacy's `readdir` would list only its directory.
                if is_hidden(name) || name.contains(&b'/') {
                    continue;
                }
                self.load_state_script(host, event, file, name);
            }
        }
    }

    /// `NPC::LoadStateScript`.
    fn load_state_script(&mut self, host: &Host, event: &'static str, file: &[u8], name: &[u8]) {
        let parsed = parse_file_name(name);
        let (quest, state) = match parsed {
            FileName::Own { quest, state } | FileName::Arg { quest, state, .. } => (quest, state),
            FileName::NoIndex | FileName::BadName => {
                let what = if parsed == FileName::NoIndex {
                    "index"
                } else {
                    "name"
                };
                tracing::warn!(
                    "invalid QUEST STATE {what} [object/{}]",
                    String::from_utf8_lossy(file)
                );
                return;
            }
        };
        let Some(quest_index) = host.quest_index(quest) else {
            tracing::warn!(
                "cannot find quest index for {} (object/{})",
                String::from_utf8_lossy(quest),
                String::from_utf8_lossy(file)
            );
            return;
        };
        // `GetQuestStateIndex`: a state the quest does not have is state 0.
        let state = usize::try_from(quest_index - 1)
            .ok()
            .and_then(|position| host.quests().get(position))
            .and_then(|quest| quest.states.get(state))
            .copied()
            .unwrap_or(0);
        let FileName::Arg { index, kind, .. } = parsed else {
            self.own
                .entry(event)
                .or_default()
                .entry(quest_index)
                .or_default()
                .entry(state)
                .or_default()
                .push(file.to_vec());
            return;
        };
        let Some(index) = arg_index(index) else {
            tracing::warn!(
                "QUEST chat menu index {index} is out of range [object/{}]",
                String::from_utf8_lossy(file)
            );
            return;
        };
        let entries = self
            .own_arg
            .entry(event)
            .or_default()
            .entry(quest_index)
            .or_default()
            .entry(state)
            .or_default();
        if entries.len() <= index {
            entries.resize(index + 1, ArgFiles::default());
        }
        let entry = &mut entries[index];
        match kind {
            b"when" => entry.when.push(file.to_vec()),
            b"arg" => entry.arg = Some(file.to_vec()),
            b"script" => {
                entry.script.push(file.to_vec());
                entry.owner = Some((quest_index, state));
            }
            _ => {}
        }
    }

    /// Compiles the files: a single file is the host's chunk, several are one chunk of their
    /// bytes in order, as legacy appended them.
    fn build(self, host: &Host) -> Npc {
        let own = self
            .own
            .into_iter()
            .map(|(event, quests)| {
                let quests = quests
                    .into_iter()
                    .map(|(quest, states)| {
                        let states = states
                            .into_iter()
                            .map(|(state, files)| (state, script(host, &files)))
                            .collect();
                        (quest, states)
                    })
                    .collect();
                (event, quests)
            })
            .collect();
        let own_arg = self
            .own_arg
            .into_iter()
            .map(|(event, quests)| {
                let quests = quests
                    .into_iter()
                    .map(|(quest, states)| {
                        let states = states
                            .into_iter()
                            .map(|(state, entries)| {
                                let entries = entries
                                    .into_iter()
                                    .map(|files| {
                                        let (quest, state) = files.owner.unwrap_or((0, 0));
                                        ArgScript {
                                            when: code(host, &files.when),
                                            arg: files.arg,
                                            script: script(host, &files.script),
                                            quest,
                                            state,
                                        }
                                    })
                                    .collect();
                                (state, entries)
                            })
                            .collect();
                        (quest, states)
                    })
                    .collect();
                (event, quests)
            })
            .collect();
        Npc { own, own_arg }
    }
}

/// The chunk of `files`, `None` when they hold no bytes.
fn code(host: &Host, files: &[Vec<u8>]) -> Option<Code> {
    if let [file] = files {
        if let Some(chunk) = host.chunk(file) {
            return Some(Ok(chunk.clone()));
        }
    }
    let bytes: Vec<u8> = files
        .iter()
        .filter_map(|file| host.objects().get(file))
        .flatten()
        .copied()
        .collect();
    if bytes.is_empty() {
        return None;
    }
    let name = format!("@object/{}", String::from_utf8_lossy(&files[0]));
    Some(host::load_chunk(host.lua(), &bytes, &name))
}

/// The chunk of a script's `files`; with no bytes it is an empty chunk, which ends at once.
fn script(host: &Host, files: &[Vec<u8>]) -> Code {
    code(host, files).unwrap_or_else(|| host::load_chunk(host.lua(), b"", "=empty"))
}

/// The NPCs of `questnpc.txt` (`questmanager.cpp:114-160`), each a number and a name on one
/// line. A line whose name is a number other than 0 (by `strtol`) names no NPC.
fn parse_questnpc(text: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let mut npcs = Vec::new();
    let mut rest = text;
    let mut line = 0u32;
    loop {
        // `inf >> vnum` skips white space, line ends included, and fails on anything but digits.
        let start = rest
            .iter()
            .position(|byte| !is_c_space(*byte))
            .unwrap_or(rest.len());
        rest = &rest[start..];
        let digits = rest
            .iter()
            .position(|byte| !byte.is_ascii_digit())
            .unwrap_or(rest.len());
        line += 1;
        let Some(vnum) = std::str::from_utf8(&rest[..digits])
            .ok()
            .and_then(|digits| digits.parse::<u32>().ok())
        else {
            break;
        };
        rest = &rest[digits..];
        // `getline` reads the rest of the line.
        let end = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(rest.len());
        let name = trim_c_space(&rest[..end]);
        rest = rest.get(end + 1..).unwrap_or_default();
        if name.is_empty() {
            tracing::warn!("QUEST questnpc.txt:{line}:npc name error");
            continue;
        }
        let number = strtol(name).to_le_bytes();
        if i32::from_le_bytes([number[0], number[1], number[2], number[3]]) != 0 {
            continue;
        }
        npcs.push((vnum, name.to_vec()));
    }
    npcs
}

/// C's `isspace` in the C locale.
/// `CQuestManager::ReadQuestCategoryToDict` (`G/questmanager.cpp:320-365`, with `_QR_MS_`
/// defined): each line that is not empty is split at white space, and its first word, read
/// by `atoi`, is the category of the quest its second word names. A later line for a quest
/// replaces an earlier one, and a name no quest has is logged and skipped. The owner's file
/// is empty, so every quest is in category 0.
fn parse_questcategory(text: &[u8], indices: &BTreeMap<Vec<u8>, u32>) -> BTreeMap<u16, u32> {
    let mut categories = BTreeMap::new();
    for line in text.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let mut words = line
            .split(|byte| is_c_space(*byte))
            .filter(|word| !word.is_empty());
        let (Some(number), Some(name)) = (words.next(), words.next()) else {
            tracing::warn!(
                line = %String::from_utf8_lossy(line),
                "QUEST questcategory.txt line without a category and a quest; skipped"
            );
            continue;
        };
        // `atoi` is `(int)strtol` with a 32-bit `long`, and the map keeps it unsigned.
        let category = strtol(number).clamp(i64::from(i32::MIN), i64::from(i32::MAX));
        let category =
            u32::from_le_bytes(i32::try_from(category).unwrap_or_default().to_le_bytes());
        if let Some(index) = indices.get(name) {
            let [low, high, ..] = index.to_le_bytes();
            categories.insert(u16::from_le_bytes([low, high]), category);
        } else {
            tracing::warn!(
                "QUEST coult not find QuestIndex for name Quest: {}({category})",
                String::from_utf8_lossy(name)
            );
        }
    }
    categories
}

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn trim_c_space(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !is_c_space(*byte))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !is_c_space(*byte))
        .map_or(start, |last| last + 1);
    &bytes[start..end.max(start)]
}

/// `strtol(text, NULL, 10)` on a 64-bit `long`: white space, a sign and the digits, 0 without
/// digits, and the nearest end of the range when the number is past it.
fn strtol(text: &[u8]) -> i64 {
    let start = text
        .iter()
        .position(|byte| !is_c_space(*byte))
        .unwrap_or(text.len());
    let text = &text[start..];
    let (negative, digits) = match text.first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let mut value: i64 = 0;
    for digit in digits.iter().take_while(|byte| byte.is_ascii_digit()) {
        let digit = i64::from(*digit - b'0');
        value = if negative {
            value.saturating_mul(10).saturating_sub(digit)
        } else {
            value.saturating_mul(10).saturating_add(digit)
        };
    }
    value
}

/// The NPCs `RegisterNPCVnum` found a directory for: every top directory of `object/` that is
/// a number as `%u` prints it.
fn numeric_dirs(host: &Host) -> BTreeSet<u32> {
    host.objects()
        .keys()
        .filter_map(|file| numeric_dir(file))
        .collect()
}

/// The NPC whose directory holds `file`: its top directory when that is a number as `%u` prints
/// it.
fn numeric_dir(file: &[u8]) -> Option<u32> {
    let dir = &file[..file.iter().position(|byte| *byte == b'/')?];
    let vnum: u32 = std::str::from_utf8(dir).ok()?.parse().ok()?;
    (vnum.to_string().as_bytes() == dir).then_some(vnum)
}

/// A file `NPC::Set` skips: a hidden one, or one whose name starts with `CVS` in any case
/// (`questnpc.cpp:60`, `:63`).
fn is_hidden(name: &[u8]) -> bool {
    name.first() == Some(&b'.')
        || name
            .get(..3)
            .is_some_and(|cvs| cvs.eq_ignore_ascii_case(b"CVS"))
}

/// A chat menu index the loader accepts: `0..=65535`.
fn arg_index(index: i64) -> Option<usize> {
    usize::try_from(index)
        .ok()
        .filter(|_| (0..=MAX_ARG_INDEX).contains(&index))
}

/// A character's quests (`quest::PC`).
#[derive(Debug, Default)]
struct Pc {
    /// `m_QuestInfo`: the state each quest the character ran is in.
    quests: BTreeMap<u32, i32>,
    /// `m_FlagMap`.
    flags: BTreeMap<Vec<u8>, i32>,
    /// `m_RunningQuestState`: the suspended script.
    running: Option<Running>,
    /// `m_stCurQuest`: the quest that ran last.
    current: Vec<u8>,
    /// `m_bShouldSendDone`: the client answered, so a bare `[DONE]` must close its window.
    send_done: bool,
    /// `SetCurrentQuestStateName`: a `set_state` the running script asked for in a step
    /// that suspended, carried until its thread ends ([`Run::end_running`]) so the
    /// bridge's `&mut Cx` never reaches the `Pc`.
    pending_state: Option<Vec<u8>>,
    /// `m_QuestStateChange`: the other quests' reserved moves, carried like
    /// `pending_state` until the thread ends.
    pending_others: Vec<(u32, Vec<u8>)>,
    npc_vid: u32,
    /// `m_pQuestItem` (`G/char.cpp:7620-7633`): the ID of the item whose `use` ran last, or 0.
    /// Legacy keeps a raw pointer that nothing clears when the item is destroyed, so a later
    /// `item.get_vnum` reads freed memory (a Defect); the Rewrite keeps the ID and asks the
    /// game whether the character still holds it.
    item: u32,
    /// `m_bConfirmWait` and `m_dwWaitConfirmFromPID` (`G/questpc.cpp:22`, `:141-144`): the PID
    /// whose answer the confirm waits for (`Some(0)` when the target was not a PC).
    confirm_wait: Option<u32>,
}

/// A running script (`QuestState`).
#[derive(Debug)]
struct Running {
    thread: mlua::Result<Thread>,
    /// The number of the quest the script belongs to, which its `set_state` moves.
    quest: u32,
    suspend: Suspend,
    /// `chat_scripts`: the entries of the chat menu this script shows.
    chat: Vec<ArgScript>,
    state: i32,
    /// What the client's quest window shows of the quest, and what the script changed. It
    /// lives here while the script waits, and in [`Cx::window`] while a step runs.
    window: Window,
}

/// The part of a running script's `QuestState` the client's quest window reads, with the
/// bits of `m_iSendToClient` that say what the script changed. Each run starts a new one
/// (`OpenState`, and `PC::SetQuest` with `m_iSendToClient = 0`, `G/questpc.cpp:140-162`).
/// `m_iSendToClient` (`G/questpc.h:35-44`): which parts of the quest's window the script
/// changed, as the flag byte `SendQuestInfoPakcet` sends (`G/questpc.cpp:231-308`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct SendFlags(u8);

impl SendFlags {
    /// `QUEST_SEND_ISBEGIN`: `q.start` or `q.done` moved `bStart`.
    const BEGIN: u8 = 1 << 0;
    /// `QUEST_SEND_TITLE`: `q.set_title` set the title.
    const TITLE: u8 = 1 << 1;
    /// `QUEST_SEND_CLOCK_NAME`: `q.set_clock_name` set the clock name.
    const CLOCK_NAME: u8 = 1 << 2;
    /// `QUEST_SEND_CLOCK_VALUE`: `q.set_clock_value` set the clock value.
    const CLOCK_VALUE: u8 = 1 << 3;
    /// `QUEST_SEND_COUNTER_NAME`: `q.set_counter_name` set the counter name.
    const COUNTER_NAME: u8 = 1 << 4;
    /// `QUEST_SEND_COUNTER_VALUE`: `q.set_counter_value` set the counter value.
    const COUNTER_VALUE: u8 = 1 << 5;

    /// `SetSendFlag`: the flag of the part the script changed.
    fn set(&mut self, flag: u8) {
        self.0 |= flag;
    }

    /// Whether the flag of the part is set.
    fn has(self, flag: u8) -> bool {
        self.0 & flag != 0
    }
}

#[derive(Debug, Default)]
struct Window {
    /// `iIndex`: `PC::SetQuest` gives it the number of the quest that runs.
    index: u32,
    /// `bStart`.
    start: bool,
    /// `_title`.
    title: Vec<u8>,
    /// `_clock_name`.
    clock_name: Vec<u8>,
    /// `_clock_value`.
    clock_value: i32,
    /// `_counter_name`.
    counter_name: Vec<u8>,
    /// `_counter_value`.
    counter_value: i32,
    /// `m_iSendToClient`: which parts the script changed.
    send: SendFlags,
}

impl Window {
    /// What `SendQuestInfoPakcet` sends, when the script changed anything
    /// (`if (m_iSendToClient)`, `G/questpc.cpp:342-346`). The category is looked up by the
    /// index the record carries.
    fn info(&self, categories: &BTreeMap<u16, u32>) -> Option<QuestInfo> {
        if self.send == SendFlags::default() {
            return None;
        }
        let [low, high, ..] = self.index.to_le_bytes();
        let index = u16::from_le_bytes([low, high]);
        let [low, high, ..] = categories.get(&index).copied().unwrap_or(0).to_le_bytes();
        Some(QuestInfo {
            index,
            category: u16::from_le_bytes([low, high]),
            begin: self.send.has(SendFlags::BEGIN).then_some(self.start),
            title: self.send.has(SendFlags::TITLE).then(|| self.title.clone()),
            clock_name: self
                .send
                .has(SendFlags::CLOCK_NAME)
                .then(|| self.clock_name.clone()),
            clock_value: self
                .send
                .has(SendFlags::CLOCK_VALUE)
                .then_some(self.clock_value),
            counter_name: self
                .send
                .has(SendFlags::COUNTER_NAME)
                .then(|| self.counter_name.clone()),
            counter_value: self
                .send
                .has(SendFlags::COUNTER_VALUE)
                .then_some(self.counter_value),
        })
    }
}

/// What a receive-all pass does with a script that still waits for its client when it returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wait {
    /// `HandleReceiveAllEvent`: the script keeps waiting.
    Kept,
    /// `HandleReceiveAllNoWaitEvent`, which the login runs: the script is closed.
    Closed,
}

/// `suspend_state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Suspend {
    None,
    Pause,
    Select,
    Input,
    Confirm,
    SelectItem,
}

/// The dialog being built (`m_strScript`, `m_iCurrentSkin`, `m_bError`).
#[derive(Debug)]
struct Output {
    script: Vec<u8>,
    skin: u8,
    error: bool,
}

impl Default for Output {
    fn default() -> Self {
        Output {
            script: Vec::new(),
            skin: QUEST_SKIN_NORMAL,
            error: false,
        }
    }
}

/// What the bridged API reaches during one call into the manager.
struct Cx<'g> {
    output: Output,
    game: &'g mut dyn Game,
    event_flags: &'g BTreeMap<Vec<u8>, i32>,
    npc_ids: &'g BTreeMap<Vec<u8>, u32>,
    /// The number of every quest, by name (`m_hmQuestName`,
    /// `G/questmanager.cpp:1284-1289`), which `q.getcurrentquestindex` answers.
    quest_indices: &'g BTreeMap<Vec<u8>, u32>,
    npc_vid: u32,
    /// `m_dwServerTimerArg`, which `get_server_timer_arg` pushes.
    timer_arg: u32,
    /// `m_pSelectedDungeon`'s private index, which the `d.*` names read when no
    /// character runs, and which `d.select` writes (read back after the step).
    selected: Option<u32>,
    /// The quest that runs, for error messages (`GetCurrentQuestName`).
    quest: Vec<u8>,
    /// What `set_state` asked for in this step, which [`Run::end_running`] moves the `Pc` to
    /// when the script ends, or the `Pc` carries while the script waits.
    pending_state: Option<Vec<u8>>,
    /// What `set_quest_state` reserved for other quests in this step, each quest's
    /// number and its state name, which `end_running` moves after the running quest
    /// (`DoQuestStateChange`, `G/questpc.cpp:377-419`), or the `Pc` carries while the
    /// script waits.
    pending_others: Vec<(u32, Vec<u8>)>,
    /// Every PC's flag map, borrowed from the manager for Lua's length: the bridge
    /// reads and writes the `current` PC's map, and a `q.begin_other_pc_block`
    /// switches `current` to the blocked PC's. `None` only in the direct-call
    /// probe, which answers through the `Game` alone.
    borrowed: Option<&'g mut BTreeMap<u32, BTreeMap<Vec<u8>, i32>>>,
    /// The PID whose flag map the bridge reads: the step's root, or the innermost
    /// blocked PC's (`m_pCurrentPC->GetID()`, `G/questmanager.cpp:2051-2093`).
    current: u32,
    /// `m_vecPCStack`: the PIDs the open blocks pushed, oldest first.
    stack: Vec<u32>,
    /// The quest window of the script whose step runs, which `q.set_title`, `q.start` and
    /// `q.done` change; [`Run::resume`] moves it in from [`Running::window`] and back.
    window: Window,
    /// Each quest's category (`GetQuestCategoryByQuestIndex`).
    categories: &'g BTreeMap<u16, u32>,
    /// The quest item's ID ([`Pc::item`]), which `item.get_vnum` reads.
    item: u32,
    /// What a `confirm` yield asked the waiter's `confirm_wait` to become, carried until
    /// the resume moves it into the `Pc`, like `pending_state`.
    confirm_wait: Option<u32>,
}

impl Cx<'_> {
    /// The live flag map for a read: the `current` PC's, or `None` when the
    /// bridge never touched it.
    fn flag_map(&self) -> Option<&BTreeMap<Vec<u8>, i32>> {
        self.borrowed.as_deref()?.get(&self.current)
    }

    /// The live flag map for a write: the `current` PC's, made when the bridge
    /// first touches it (`GetPCForce`, `G/questmanager.cpp:1073-1085`).
    fn flags(&mut self) -> Option<&mut BTreeMap<Vec<u8>, i32>> {
        let current = self.current;
        Some(self.borrowed.as_deref_mut()?.entry(current).or_default())
    }
}

/// One call into the manager. No `Cx` borrow is held across a call into Lua, which may call
/// back through the bridge.
struct Run<'r, 'g> {
    host: &'r Host,
    cx: &'r RefCell<Cx<'g>>,
    /// `m_mapNPC[QUEST_NO_NPC]`: the scripts no NPC owns, which a state change runs.
    no_npc: Option<&'r Npc>,
    /// How many state changes run inside one another now.
    changes: Cell<u8>,
}

impl Run<'_, '_> {
    fn lua(&self) -> &Lua {
        self.host.lua()
    }

    /// Runs `op` with [`BRIDGE`] set to a function that lives no longer than this call.
    fn bridged<R: Default>(&self, op: impl FnOnce() -> R) -> R {
        let lua = self.lua();
        let cx = self.cx;
        let mut result = None;
        let scoped = lua.scope(|scope| {
            let scope: &mlua::Scope = scope;
            let bridge =
                scope.create_function(move |lua, (name, args): (mlua::String, MultiValue)| {
                    let mut cx = cx
                        .try_borrow_mut()
                        .map_err(|_| mlua::Error::runtime("the quest bridge is busy"))?;
                    api(lua, &mut cx, &name.as_bytes(), &args)
                })?;
            lua.set_named_registry_value(BRIDGE, bridge)?;
            result = Some(op());
            lua.unset_named_registry_value(BRIDGE)
        });
        if let Err(error) = scoped {
            tracing::error!(%error, "QUEST the bridge failed");
        }
        result.unwrap_or_default()
    }

    /// `GetQuestNameByIndex`: the empty name for a number no quest has (legacy asserts).
    fn quest_name(&self, quest: u32) -> Vec<u8> {
        usize::try_from(quest)
            .ok()
            .and_then(|quest| quest.checked_sub(1))
            .and_then(|position| self.host.quests().get(position))
            .map(|quest| quest.name.clone())
            .unwrap_or_default()
    }

    /// `GetQuestStateName`: the empty name for a state the quest does not have.
    fn state_name(&self, quest: &[u8], state: i32) -> Vec<u8> {
        self.host
            .quests()
            .iter()
            .find(|candidate| candidate.name == quest)
            .and_then(|quest| quest.states.iter().find(|(_, number)| **number == state))
            .map(|(name, _)| name.clone())
            .unwrap_or_default()
    }

    /// `CanStartQuest` (`questmanager.cpp:1047`): the quest's `begin_condition`, if it has one.
    fn can_start(&self, quest: u32) -> bool {
        let file = [&b"begin_condition/"[..], &self.quest_name(quest)].concat();
        let Some(condition) = self.host.chunk(&file) else {
            return true;
        };
        match condition.call::<MultiValue>(()) {
            Ok(values) => values.back().is_some_and(truthy),
            Err(error) => {
                tracing::warn!(%error, "QUEST begin_condition failed");
                false
            }
        }
    }

    /// `ScriptToString` (`questlua.cpp:33`): the text of a chat menu entry's argument.
    fn script_to_string(&self, arg: Option<&[u8]>) -> Vec<u8> {
        let Some(chunk) = arg.and_then(|file| self.host.chunk(file)) else {
            return Vec::new();
        };
        match chunk.call::<MultiValue>(()) {
            Ok(values) => values
                .back()
                .and_then(|value| host::c_string(self.lua(), value.clone()).ok().flatten())
                .unwrap_or_default(),
            Err(error) => {
                tracing::warn!(%error, "LUA ScriptRunError");
                Vec::new()
            }
        }
    }

    /// `NPC::OnChat` (`questnpc.cpp:909`): the menu of every entry the character may pick.
    fn on_chat(&self, pc: &mut Pc, npc: &Npc) -> bool {
        if pc.running.is_some() {
            return false;
        }
        let Some(quests) = npc.own_arg.get("chat") else {
            return false;
        };
        let mut avail: Vec<&ArgScript> = Vec::new();
        for (quest, states) in quests {
            let entries = match pc.quests.get(quest) {
                Some(state) => states.get(state),
                None if self.can_start(*quest) => states.get(&0),
                None => None,
            };
            for entry in entries.into_iter().flatten() {
                if is_true(entry.when.as_ref()) {
                    avail.push(entry);
                }
            }
        }
        if avail.is_empty() {
            return false;
        }
        let mut source = b"select(".to_vec();
        for (index, entry) in avail.iter().enumerate() {
            source.extend_from_slice(if index == 0 { b"\"" } else { b",\"" });
            source.extend(self.script_to_string(entry.arg.as_deref()));
            source.push(b'"');
        }
        source.extend_from_slice(b", 'Inchide')");
        let code = host::load_chunk(self.lua(), &source, "QUEST_CHAT_TEMP_QUEST");
        let chat = avail.into_iter().cloned().collect();
        self.execute(pc, 0, CHAT_TEMP_QUEST.to_vec(), 0, code, chat);
        true
    }

    /// `NPC::OnTarget` (`G/questnpc.cpp:206-264`) on the no-NPC scripts: the first `target`
    /// entry of quest `quest` at the character's state of it whose argument begins with `name`
    /// (`strncmp` over the name's length, so `boss` also answers `bossy.click`), names `verb`
    /// after its first dot, and whose condition holds, runs at that state. Answers whether one
    /// matched; nothing matches for a quest the character never ran.
    ///
    /// While another script waits for its client the match runs nothing but still answers
    /// `true`. Legacy runs it, and `PC::SetQuest` drops the waiting script's state without
    /// closing it (`G/questpc.cpp:140-164`), a Defect.
    fn on_target(&self, pc: &mut Pc, quest: u32, name: &[u8], verb: &[u8]) -> bool {
        let Some(&state) = pc.quests.get(&quest) else {
            return false;
        };
        let Some(entries) = self
            .no_npc
            .and_then(|npc| npc.own_arg.get("target"))
            .and_then(|quests| quests.get(&quest))
            .and_then(|states| states.get(&state))
        else {
            return false;
        };
        for entry in entries {
            let arg = self.target_arg(entry.arg.as_deref());
            if !arg.starts_with(name) {
                continue;
            }
            let Some(dot) = arg.iter().position(|byte| *byte == b'.') else {
                continue;
            };
            if arg[dot + 1..] != *verb || !is_true(entry.when.as_ref()) {
                continue;
            }
            if pc.running.is_some() {
                tracing::warn!(quest, "QUEST a target script waits for the running one");
                return true;
            }
            let name = self.quest_name(quest);
            self.execute(pc, quest, name, state, entry.script.clone(), Vec::new());
            return true;
        }
        false
    }

    /// A `target` entry's argument as `NPC::LoadStateScript` keeps it (`G/questnpc.cpp:171-180`):
    /// the first line of its `.arg` file, read as the C string `OnTarget` compares.
    fn target_arg(&self, file: Option<&[u8]>) -> &[u8] {
        let Some(text) = file.and_then(|file| self.host.objects().get(file)) else {
            return &[];
        };
        let end = text
            .iter()
            .position(|byte| *byte == b'\n' || *byte == 0)
            .unwrap_or(text.len());
        &text[..end]
    }

    /// `NPC::ExecuteEventScript` (`G/questnpc.cpp:966-988`): the named quest's script for
    /// the event at the given state, and nothing else.
    ///
    /// The legacy's letter and button name one quest (`OnLetter`, `OnButton`), where the
    /// generic [`Run::handle_event`] runs every quest of the NPC.
    fn event_script(&self, pc: &mut Pc, npc: &Npc, event: &str, quest: u32, state: i32) -> bool {
        let Some(code) = npc
            .own
            .get(event)
            .and_then(|quests| quests.get(&quest))
            .and_then(|states| states.get(&state))
        else {
            return false;
        };
        let name = self.quest_name(quest);
        self.execute(pc, quest, name, state, code.clone(), Vec::new());
        true
    }

    /// `NPC::HandleReceiveAllEvent` and `NPC::HandleReceiveAllNoWaitEvent`
    /// (`G/questnpc.cpp:621-646,726-758`) over `MatchingQuest` (`G/questnpc.h:112-139`):
    /// in quest order, each quest's script at the character's state of it, or its start
    /// state's when the character never ran it and may start it. Unlike
    /// [`Run::handle_event`], which runs every match before every start, the scripts run
    /// in one pass in quest order.
    ///
    /// With [`Wait::Closed`] a script still waiting for its client when it returns is
    /// closed (`CloseState` and `EndRunning`), and a running script does not stop the pass:
    /// legacy's no-wait variant has its `IsRunning` check commented out.
    fn handle_receive_all(&self, pc: &mut Pc, npc: &Npc, event: &str, wait: Wait) -> bool {
        if wait == Wait::Kept && pc.running.is_some() {
            return false;
        }
        let Some(quests) = npc.own.get(event) else {
            return false;
        };
        let mut handled = false;
        for (quest, states) in quests {
            let (state, code) = match pc.quests.get(quest) {
                Some(state) => match states.get(state) {
                    Some(code) => (*state, code),
                    None => continue,
                },
                None => match states.get(&0) {
                    Some(code) if self.can_start(*quest) => (0, code),
                    _ => continue,
                },
            };
            handled = true;
            let name = self.quest_name(*quest);
            let waits = self.execute(pc, *quest, name.clone(), state, code.clone(), Vec::new());
            if waits && wait == Wait::Closed {
                tracing::warn!(
                    "QUEST NOT END RUNNING on Login/Logout - {}",
                    String::from_utf8_lossy(&name)
                );
                self.end_running(pc);
            }
        }
        handled
    }

    /// `NPC::HandleEvent` (`questnpc.cpp:476`): the script of each quest's current state, then
    /// the start state of each quest the character never ran.
    fn handle_event(&self, pc: &mut Pc, npc: &Npc, event: &str) -> bool {
        if pc.running.is_some() {
            return false;
        }
        let Some(quests) = npc.own.get(event) else {
            return false;
        };
        let mut matched = Vec::new();
        let mut missed = Vec::new();
        for (quest, states) in quests {
            match pc.quests.get(quest) {
                Some(state) => {
                    if let Some(code) = states.get(state) {
                        matched.push((*quest, *state, code.clone()));
                    }
                }
                None => {
                    if let Some(code) = states.get(&0).filter(|_| self.can_start(*quest)) {
                        missed.push((*quest, 0, code.clone()));
                    }
                }
            }
        }
        let handled = !matched.is_empty() || !missed.is_empty();
        for (quest, state, code) in matched.into_iter().chain(missed) {
            let name = self.quest_name(quest);
            self.execute(pc, quest, name, state, code, Vec::new());
        }
        handled
    }

    /// `ExecuteQuestScript` (`questmanager.cpp:1772`) with `PC::SetQuest` (`questpc.cpp:140`).
    fn execute(
        &self,
        pc: &mut Pc,
        quest: u32,
        name: Vec<u8>,
        state: i32,
        code: Code,
        chat: Vec<ArgScript>,
    ) -> bool {
        let thread = code.and_then(|function| self.lua().create_thread(function));
        // `PC::SetQuest`: `iIndex` is `GetQuestIndexByName`, 0 for the chat menu's quest.
        let index = self.host.quest_indices().get(&name).copied().unwrap_or(0);
        pc.quests.insert(quest, state);
        pc.flags.insert([&name[..], b".__status"].concat(), state);
        pc.send_done = false;
        self.cx.borrow_mut().quest.clone_from(&name);
        pc.current = name;
        pc.pending_state = None;
        pc.running = Some(Running {
            thread,
            quest,
            suspend: Suspend::None,
            chat,
            state,
            window: Window {
                index,
                ..Window::default()
            },
        });
        self.resume(pc, Ok(MultiValue::new()))
    }

    /// `EndRunning` (`G/questpc.cpp:310-374`), which every end of a script runs, a
    /// refusal's and an error's too: the quest's window goes to the client when the script
    /// changed it (`SendQuestInfoPakcet`, `:342-346`), and then the quest of the script
    /// moves to the state its last `set_state` named, whichever step asked for it
    /// (`quest_setstate` writes the running state at once, `G/questlua_quest.cpp:112-146`),
    /// running the scripts of the change ([`Run::goto_state`]), and then the reserved
    /// other quests move in order (`DoQuestStateChange`, `G/questpc.cpp:377-419`).
    fn end_running(&self, pc: &mut Pc) {
        let asked = self.cx.borrow_mut().pending_state.take();
        let pending = asked.or_else(|| pc.pending_state.take());
        pc.pending_state = None;
        let Some(running) = pc.running.take() else {
            return;
        };
        {
            let mut cx = self.cx.borrow_mut();
            if let Some(info) = running.window.info(cx.categories) {
                cx.game.quest_info(&info);
            }
        }
        if let Some(state) = pending {
            self.goto_state(pc, running.quest, running.state, &state);
        }
        let mut others = std::mem::take(&mut pc.pending_others);
        others.append(&mut self.cx.borrow_mut().pending_others);
        for (quest, state) in others {
            if quest == 0 {
                continue;
            }
            let last = pc.quests.get(&quest).copied().unwrap_or(0);
            self.goto_state(pc, quest, last, &state);
        }
    }

    /// `RunState` (`questlua.cpp:995`), and the `CloseState` and `EndRunning` every caller did
    /// when it returned false.
    fn resume(&self, pc: &mut Pc, args: mlua::Result<MultiValue>) -> bool {
        let Some(running) = pc.running.as_mut() else {
            return false;
        };
        {
            let mut cx = self.cx.borrow_mut();
            cx.output.error = false;
            cx.window = std::mem::take(&mut running.window);
        }
        let resumed = match &running.thread {
            Ok(thread) => args.and_then(|args| thread.resume::<MultiValue>(args)),
            Err(error) => Err(error.clone()),
        };
        // The window goes back to the script before any end of it reads it.
        running.window = std::mem::take(&mut self.cx.borrow_mut().window);
        let step = match resumed {
            Ok(values) if values.is_empty() => {
                self.goto_end(pc);
                self.end_running(pc);
                return false;
            }
            Ok(values) => self.suspension(&values),
            Err(error) => {
                // A refusal is not a script error: the legacy's API worked, and the script
                // ended as a script that says nothing ends. So the end is silent: the bare
                // `[DONE]` is swallowed (`SendScript`, `G/questmanager.cpp:1122-1130`), and
                // the client sees nothing where the legacy's script saw nothing either.
                if is_refusal(&error) {
                    tracing::debug!("QUEST refused: {error}");
                    self.goto_end(pc);
                    self.end_running(pc);
                    return false;
                }
                tracing::warn!("LUA_ERROR: {error}");
                None
            }
        };
        let Some((suspend, text)) = step else {
            // `WriteRunningStateToSyserr`, `SetError` and `GotoEndState`.
            let state = pc.running.as_ref().map_or(0, |running| running.state);
            tracing::warn!(
                "LUA_ERROR: quest {}.{} click",
                String::from_utf8_lossy(&pc.current),
                String::from_utf8_lossy(&self.state_name(&pc.current, state))
            );
            self.cx.borrow_mut().output.error = true;
            self.goto_end(pc);
            self.end_running(pc);
            return false;
        };
        self.cx.borrow_mut().output.script.extend(text);
        if let Some(running) = pc.running.as_mut() {
            running.suspend = suspend;
        }
        if let Some(state) = self.cx.borrow_mut().pending_state.take() {
            pc.pending_state = Some(state);
        }
        pc.pending_others
            .append(&mut self.cx.borrow_mut().pending_others);
        if let Some(wait) = self.cx.borrow_mut().confirm_wait.take() {
            pc.confirm_wait = Some(wait);
        }
        self.send_script(pc);
        true
    }

    /// What the values a script yielded ask for, with the text that goes with it: `select`'s
    /// menu (`GotoSelectState`), `wait`'s `[NEXT]` or `input`'s `[INPUT]`. `None` is an error.
    fn suspension(&self, values: &MultiValue) -> Option<(Suspend, Vec<u8>)> {
        let first = values.front()?;
        let what = host::c_string(self.lua(), first.clone()).ok().flatten()?;
        match &what[..] {
            b"select" => {
                let Some(Value::Table(choices)) = values.back() else {
                    tracing::warn!("QUEST select without a table of choices");
                    return None;
                };
                let n = host::getn(self.lua(), choices).ok()?;
                let mut text = b"[QUESTION ".to_vec();
                for i in 1..=n {
                    let choice: Value = choices.raw_get(i).ok()?;
                    let Some(label) = host::c_string(self.lua(), choice.clone()).ok().flatten()
                    else {
                        tracing::warn!("SELECT wrong data {}", choice.type_name());
                        continue;
                    };
                    if i != 1 {
                        text.push(b'|');
                    }
                    text.extend_from_slice(i.to_string().as_bytes());
                    text.push(b';');
                    text.extend(label);
                }
                text.push(b']');
                Some((Suspend::Select, text))
            }
            b"wait" => Some((Suspend::Pause, b"[NEXT]".to_vec())),
            b"input" => Some((Suspend::Input, b"[INPUT]".to_vec())),
            b"select_item" => Some((Suspend::SelectItem, b"[SELECT_ITEM]".to_vec())),
            b"confirm" => {
                // `GotoConfirmState` (`G/questlua.cpp:906-943`): the stack above `confirm`
                // holds the vid, the message and the timeout.
                let mut rest = values.into_iter();
                let _confirm = rest.next();
                let vid = rest.next();
                let msg = rest.next();
                let timeout = rest.next();
                let vid = vid.and_then(|vid| host::c_int(self.lua(), vid.clone()).ok());
                let msg = msg
                    .and_then(|msg| host::c_string(self.lua(), msg.clone()).ok().flatten())
                    .unwrap_or_default();
                let timeout = timeout
                    .and_then(|timeout| host::c_int(self.lua(), timeout.clone()).ok())
                    .unwrap_or(0);
                let vid = u32::try_from(vid.unwrap_or(0)).unwrap_or(0);
                let mut cx = self.cx.borrow_mut();
                let to_pid = cx.game.pid_of(vid);
                let waiter = cx.current;
                if let Some(to_pid) = to_pid {
                    cx.game.confirm_to(to_pid, &msg, timeout, waiter);
                }
                cx.game.arm_confirm_timeout(waiter, timeout);
                cx.confirm_wait = Some(to_pid.unwrap_or(0));
                let mut text = b"[CONFIRM_WAIT timeout;".to_vec();
                text.extend_from_slice(timeout.to_string().as_bytes());
                text.push(b']');
                Some((Suspend::Confirm, text))
            }
            _ => None,
        }
    }

    /// `GotoEndState`.
    fn goto_end(&self, pc: &mut Pc) {
        self.cx
            .borrow_mut()
            .output
            .script
            .extend_from_slice(b"[DONE]");
        self.send_script(pc);
    }

    /// `Confirm`'s close of the waiting window (`G/questmanager.cpp:224-226`):
    /// `[END_CONFIRM_WAIT]` in `QUEST_SKIN_NOWINDOW`, sent before the script resumes.
    fn end_confirm_wait(&self, pc: &mut Pc) {
        {
            let mut cx = self.cx.borrow_mut();
            cx.output.script.extend_from_slice(b"[END_CONFIRM_WAIT]");
            cx.output.skin = QUEST_SKIN_NOWINDOW;
        }
        self.send_script(pc);
    }
    /// `SetCurrentQuestStateName` (`questlua_quest.cpp:112`): the quest of the finished script
    /// moves to the named state. A name the quest does not have is legacy's index 0
    /// (`GetQuestStateIndex`), and the status flag follows the state.
    ///
    /// When the state is not `last`, the one the script started in (`m_iLastState`),
    /// `EndRunning` runs the change (`G/questpc.cpp:352-371`): the old state's `leave` script
    /// runs at `last`, then the quest and its flag move on for the new state's `enter` script,
    /// and the new state's `letter` script runs when the flag still names the new state, which
    /// an `enter` script that moved the quest on again changed. Each script runs as
    /// `NPC::ExecuteEventScript` runs it (`G/questnpc.cpp:191-204`), nested inside this end,
    /// so a script that ends runs its own `EndRunning` first. Legacy also puts the flag back to
    /// `last` before the `leave`; `execute` writes it for the script it runs, as
    /// `PC::SetQuest` does (`:140-164`), and nothing reads it when there is no script, so the
    /// port has no such write.
    fn goto_state(&self, pc: &mut Pc, quest: u32, last: i32, state: &[u8]) {
        let at = usize::try_from(quest.saturating_sub(1)).unwrap_or(usize::MAX);
        let number = self
            .host
            .quests()
            .get(at)
            .and_then(|quest| quest.states.get(state))
            .copied()
            .unwrap_or(0);
        pc.quests.insert(quest, number);
        let name = self.quest_name(quest);
        let status = [&name[..], b".__status"].concat();
        pc.flags.insert(status.clone(), number);
        if number == last {
            return;
        }
        let depth = self.changes.get();
        if depth >= STATE_CHANGE_DEPTH {
            tracing::error!(
                "QUEST {} changes state {depth} deep; its scripts do not run",
                String::from_utf8_lossy(&name)
            );
            return;
        }
        self.changes.set(depth + 1);
        self.state_script(pc, "leave", quest, last);
        pc.quests.insert(quest, number);
        pc.flags.insert(status.clone(), number);
        self.state_script(pc, "enter", quest, number);
        // The status flag is written only here and in `execute`, both into `pc.flags`; the
        // bridge's half of the flags (`Cx::quest_flags`) never holds it.
        if pc.flags.get(&status).copied().unwrap_or(0) == number {
            self.state_script(pc, "letter", quest, number);
        }
        self.changes.set(depth);
    }

    /// `CQuestManager::EnterState`, `LeaveState` and `Letter` (`G/questmanager.cpp:474-514`):
    /// the no-NPC script of quest `quest` for `event` at `state`, when there is one.
    fn state_script(&self, pc: &mut Pc, event: &str, quest: u32, state: i32) -> bool {
        self.no_npc
            .is_some_and(|npc| self.event_script(pc, npc, event, quest, state))
    }

    /// `SendScript`: a bare `[DONE]` or `[NEXT]` closes the window once the client has
    /// answered, and a bare `[DONE]` nobody waits for is not sent at all.
    fn send_script(&self, pc: &mut Pc) {
        let mut cx = self.cx.borrow_mut();
        let cx = &mut *cx;
        let output = &mut cx.output;
        if output.script == b"[DONE]" || output.script == b"[NEXT]" {
            let done = std::mem::take(&mut pc.send_done);
            if !done
                && output.script == b"[DONE]"
                && output.skin == QUEST_SKIN_NORMAL
                && !output.error
            {
                output.script.clear();
                output.skin = QUEST_SKIN_NORMAL;
                return;
            }
            output.skin = QUEST_SKIN_NOWINDOW;
        }
        let script = std::mem::take(&mut output.script);
        let skin = std::mem::replace(&mut output.skin, QUEST_SKIN_NORMAL);
        cx.game.script(skin, &script);
    }
}

/// Whether `error` is a refusal: the call of an API the Rewrite does not back
/// (`not_ported`, `quest/src/host.rs`). A refusal is not a script error: the legacy's API
/// worked, so the legacy's script carried on and ended as a script that says nothing ends.
fn is_refusal(error: &mlua::Error) -> bool {
    error.to_string().contains(host::NOT_PORTED)
}

/// `IsScriptTrue` (`questlua.cpp:173`): no condition is true, else the truth of its last
/// result; an error is false.
fn is_true(when: Option<&Code>) -> bool {
    let Some(when) = when else {
        return true;
    };
    let result = when
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|function| function.call::<MultiValue>(()));
    match result {
        Ok(values) => values.back().is_some_and(truthy),
        Err(error) => {
            tracing::warn!(%error, "LUA ScriptRunError");
            false
        }
    }
}

/// `lua_toboolean`.
fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Nil | Value::Boolean(false))
}

/// `combine_lua_string` (`questlua.cpp:192`): every argument that is text or a number, as
/// text, each up to its first zero byte.
fn combine(lua: &Lua, args: &MultiValue) -> mlua::Result<Vec<u8>> {
    let mut text = Vec::new();
    for arg in args {
        if let Some(part) = host::c_string(lua, arg.clone())? {
            text.extend(part);
        }
    }
    Ok(text)
}

/// `pc_give_or_drop_item` (`G/questlua_pc.cpp:540-587`), `pc.give_item2`: the item's ID, or
/// 0 when none was given.
///
/// A number, or text that reads as one, is the vnum as `(int)` then `DWORD`; other text is a
/// locale name's prefix (`ITEM_MANAGER::GetVnum`); anything else, or a name no proto has,
/// gives nothing. A positive second number is the count, rounded to even, and one that rounds
/// to 0 gives nothing; otherwise the count is 1. The rewards log and the gold-bar log are not
/// ported. The count reaches `AutoGiveItem` as a `WORD`, which legacy wraps; the Rewrite
/// stops it at 65535, which the stack limit then bounds.
fn give_item2(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue) -> mlua::Result<u32> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let vnum = if lua.coerce_number(first.clone())?.is_some() {
        let vnum = host::c_int(lua, first)?;
        Some(u32::from_le_bytes(vnum.to_le_bytes()))
    } else if let Value::String(name) = &first {
        let name = host::c_string(lua, Value::String(name.clone()))?.unwrap_or_default();
        let vnum = cx.game.item_vnum(&name);
        if vnum.is_none() {
            tracing::warn!(
                name = %String::from_utf8_lossy(&name),
                "QUEST Make item call error : wrong item name"
            );
        }
        vnum
    } else {
        tracing::warn!("QUEST Make item call error : wrong argument");
        None
    };
    let Some(vnum) = vnum else {
        return Ok(0);
    };
    let second = args.get(1).cloned().unwrap_or(Value::Nil);
    let count = match lua.coerce_number(second)? {
        Some(asked) if asked > 0.0 => {
            let count = host::c_int(lua, Value::Number(asked.round_ties_even()))?;
            if count <= 0 {
                tracing::warn!(asked, "QUEST Make item call error : wrong item count");
                return Ok(0);
            }
            count
        }
        _ => 1,
    };
    let count = u16::try_from(count).unwrap_or(u16::MAX);
    Ok(cx.game.give_item(vnum, count))
}

/// `pc_warp` (`G/questlua_pc.cpp:209-240`): the first two arguments are the point as
/// `(int)`; a third that is number-like is the private index, 0 else. `IsHack` answers
/// through the game. Arguments that are not number-like log and answer false.
fn pc_warp(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue) -> mlua::Result<bool> {
    let x = args.front().cloned().unwrap_or(Value::Nil);
    let y = args.get(1).cloned().unwrap_or(Value::Nil);
    let private = args.get(2).cloned().unwrap_or(Value::Nil);
    let (Some(_), Some(_)) = (lua.coerce_number(x.clone())?, lua.coerce_number(y.clone())?) else {
        tracing::warn!("QUEST wrong argument: pc_warp needs two numbers");
        return Ok(false);
    };
    let x = host::c_int(lua, x)?;
    let y = host::c_int(lua, y)?;
    let private = match lua.coerce_number(private.clone())? {
        Some(_) => host::c_int(lua, private)?,
        None => 0,
    };
    Ok(cx.game.warp(x, y, private))
}
/// `pc_warp_to_guild_war_observer_position` (`G/questlua_pc.cpp:406-443`):
/// `luaL_checknumber` reads both guild ids (a non-number errors `bad argument #N`),
/// an unknown guild errors `no such guild with id`, and a war with no observer
/// position errors `not under warp guild war between guild`.
fn war_observe(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue) -> mlua::Result<()> {
    let mut ids = [0_u32; 2];
    for (position, id) in ids.iter_mut().enumerate() {
        let arg = args.get(position).cloned().unwrap_or(Value::Nil);
        let Some(number) = lua.coerce_number(arg.clone())? else {
            let got = if args.len() < position + 1 {
                "no value"
            } else {
                match arg {
                    Value::Nil => "nil",
                    Value::Boolean(_) => "boolean",
                    Value::Table(_) => "table",
                    Value::Function(_) => "function",
                    Value::UserData(_) => "userdata",
                    Value::Thread(_) => "thread",
                    _ => "string",
                }
            };
            return Err(mlua::Error::runtime(format!(
                "bad argument #{} to 'pc.warp_to_guild_war_observer_position' (number                  expected, got {got})",
                position + 1,
            )));
        };
        *id = dword(number);
    }
    let [first, second] = ids;
    match cx.game.war_observe(first, second) {
        Some(_) => Ok(()),
        None if !cx.game.guild_known(first) || !cx.game.guild_known(second) => Err(
            mlua::Error::runtime(format!("no such guild with id {first} {second}")),
        ),
        None => Err(mlua::Error::runtime(format!(
            "not under warp guild war between guild {first} {second}"
        ))),
    }
}

/// `pc_ore_refine` (`G/questlua_pc.cpp:1912-1937`) and `pc_diamond_refine` (`:1889-1910`):
/// the quest item melts for the fee of the first argument and the odds of the second
/// (both `(int)`, 0 for a second that is not a number). The first must be a number or
/// the melt is refused; with no quest item it is refused too. The diamond takes no
/// stone; the ore takes the item in the base inventory cell of the third (`(int)` of a
/// number, 0 of one that is not), whose presence the world judges before it spends.
fn ore_refine(lua: &Lua, cx: &mut Cx<'_>, name: &[u8], args: &MultiValue) -> mlua::Result<bool> {
    let cost = args.front().cloned().unwrap_or(Value::Nil);
    let pct = args.get(1).cloned().unwrap_or(Value::Nil);
    if lua.coerce_number(cost.clone())?.is_none() || cx.item == 0 {
        return Ok(false);
    }
    let cost = host::c_int(lua, cost)?;
    let pct = match lua.coerce_number(pct)? {
        Some(_) => host::c_int(lua, args.get(1).cloned().unwrap_or(Value::Nil))?,
        None => 0,
    };
    let stone = if name == b"pc.ore_refine" {
        let cell = host::c_int(lua, args.get(2).cloned().unwrap_or(Value::Nil))?;
        let cell = u32::try_from(cell).unwrap_or(u32::MAX);
        cx.game.inventory_at(cell)
    } else {
        None
    };
    if name == b"pc.ore_refine" && stone.is_none() {
        return Ok(false);
    }
    Ok(cx.game.ore_refine(cx.item, cost, pct, stone))
}

/// `pc_give_or_drop_item_and_select` (`G/questlua_pc.cpp:679-729`): `give_item2`'s reads and
/// give, but nothing is pushed, and the item given becomes the quest item.
fn give_item2_select(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue) -> mlua::Result<()> {
    let given = give_item2(lua, cx, args)?;
    if given != 0 {
        cx.item = given;
    }
    Ok(())
}

/// `game_drop_item_with_ownership` (`G/questlua_game.cpp:168-213`): one to three
/// arguments makes `count` of the vnum at the character's feet (1 with one argument);
/// any other count drops nothing. A vnum that is not number-like is 0 through the
/// `(DWORD)` cast, so nothing is made. The seconds are `None` with no third number.
fn drop_with_ownership(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue) -> mlua::Result<()> {
    let top = args.len();
    if (1..=3).contains(&top) {
        let vnum = args.front().cloned().unwrap_or(Value::Nil);
        let count = args.get(1).cloned().unwrap_or(Value::Nil);
        let secs = args.get(2).cloned().unwrap_or(Value::Nil);
        let vnum = u32::from_le_bytes(host::c_int(lua, vnum)?.to_le_bytes());
        let count = if top >= 2 {
            host::c_int(lua, count)?
        } else {
            1
        };
        let secs = if top >= 3 && lua.coerce_number(secs.clone())?.is_some() {
            Some(host::c_int(lua, secs)?)
        } else {
            None
        };
        cx.game.drop_with_ownership(vnum, count, secs);
    }
    Ok(())
}

/// `pc_count_item` (`G/questlua_pc.cpp:777-799`), `pc.count_item` and `pc.countitem`: a
/// number-like last argument is the vnum, dworded. Text there makes the first argument a
/// locale name's prefix (`ITEM_MANAGER::GetVnum`), and a name no proto has counts 0. Anything
/// else counts 0. A first argument that is not text either counts 0, where legacy hands
/// `GetVnum` a null name (a Defect).
fn count_item(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue) -> mlua::Result<i32> {
    let last = args.back().cloned().unwrap_or(Value::Nil);
    if let Some(vnum) = lua.coerce_number(last.clone())? {
        return Ok(cx.game.count_item(dword(vnum)));
    }
    if !matches!(last, Value::String(_)) {
        return Ok(0);
    }
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let name = host::c_string(lua, first)?.unwrap_or_default();
    let Some(vnum) = cx.game.item_vnum(&name) else {
        tracing::warn!(
            name = %String::from_utf8_lossy(&name),
            "QUEST count_item call error : wrong item name"
        );
        return Ok(0);
    };
    Ok(cx.game.count_item(vnum))
}

/// `item_set_socket` (`G/questlua_item.cpp:125-136`): with a quest item and two number-like
/// arguments, `(int)` of each, and an index of 0 to 5 (`ITEM_SOCKET_MAX_NUM`, under
/// `ENABLE_EXTENDED_SOCKETS`) writes that socket. Anything else writes nothing, and nothing is
/// pushed.
fn set_socket(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue) -> mlua::Result<()> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let second = args.get(1).cloned().unwrap_or(Value::Nil);
    if cx.item == 0
        || lua.coerce_number(first.clone())?.is_none()
        || lua.coerce_number(second.clone())?.is_none()
    {
        return Ok(());
    }
    let index = host::c_int(lua, first)?;
    let value = host::c_int(lua, second)?;
    if let Ok(index @ 0..=5) = usize::try_from(index) {
        cx.game.set_socket(cx.item, index, value);
    }
    Ok(())
}

/// (`:3834`, `:3837`): `(int)(lua_tonumber * 10)`, and nothing pushed.
fn change_alignment(lua: &Lua, cx: &mut Cx<'_>, first: &Value) -> mlua::Result<()> {
    let amount = match lua.coerce_number(first.clone())? {
        Some(number) => host::c_int(lua, Value::Number(number * 10.0))?,
        None => 0,
    };
    cx.game.change_alignment(amount);
    Ok(())
}
/// `pc_change_money` (`G/questlua_pc.cpp:1102-1121`) under its four names (`:3832-3833`,
/// `:3835-3836`): `(long long)` of the last argument (`lua_tonumber(L, -1)`), so a missing
/// or non-number argument moves nothing. It pushes nothing.
fn change_gold(lua: &Lua, cx: &mut Cx<'_>, last: &Value) -> mlua::Result<()> {
    let amount = match lua.coerce_number(last.clone())? {
        Some(number) => {
            let truncated = number.trunc();
            if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&truncated) {
                format!("{truncated:.0}").parse().unwrap_or(i64::MIN)
            } else {
                i64::MIN
            }
        }
        None => 0,
    };
    cx.game.change_gold(amount);
    Ok(())
}
/// `pc_hide_costume` (`G/questlua_pc.cpp:3553-3600`): `(BYTE)` of the first argument is the
/// part (1-5: body, hair, sash, aura, weapon) and the second, which must be a boolean, hides
/// or shows it. A non-number part, a non-boolean hide, or a part outside 1-5 logs and
/// changes nothing. The `costume_option.hide_*` quest flag follows the bit. It pushes
/// nothing.
fn hide_costume(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue) -> mlua::Result<()> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let second = args.get(1).cloned().unwrap_or(Value::Nil);
    let Some(number) = lua.coerce_number(first)? else {
        tracing::error!("pc_hide_costume::wrong part value (1-4)");
        return Ok(());
    };
    let Value::Boolean(hide) = second else {
        tracing::error!("Invalid argument: arg2 must be boolean.");
        return Ok(());
    };
    let part = dword(number).to_le_bytes()[0];
    if !(1..=5).contains(&part) {
        tracing::error!("Invalid part");
        return Ok(());
    }
    cx.game.hide_costume(part, hide);
    let flag: &[u8] = match part {
        1 => b"costume_option.hide_body",
        2 => b"costume_option.hide_hair",
        3 => b"costume_option.hide_sash",
        4 => b"costume_option.hide_aura",
        _ => b"costume_option.hide_weapon",
    };
    let flag = flag.to_vec();
    if let Some(flags) = cx.flags() {
        if hide {
            flags.insert(flag, 1);
        } else {
            flags.remove(&flag);
        }
    }
    Ok(())
}

/// `pc_open_sash` (`G/questlua_pc.cpp:3523-3536`): the first argument must be a
/// boolean, combination (`true`) or absorption; anything else logs and changes
/// nothing. It pushes nothing.
fn open_sash(cx: &mut Cx<'_>, args: &MultiValue) {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let Value::Boolean(combination) = first else {
        tracing::error!("Invalid argument: arg1 must be boolean");
        return;
    };
    cx.game.open_sash(combination);
}

/// `pc_open_changelook` (`G/questlua_pc.cpp:3540-3549`): no argument is read.
/// It pushes nothing.
fn open_changelook(cx: &mut Cx<'_>) {
    cx.game.open_changelook();
}

/// `pc_get_wear` (`G/questlua_pc.cpp:1052-1071`): the vnum worn in `(BYTE)` of the first
/// argument, nil when the cell is out of range or empty. A non-number argument logs and
/// pushes nothing.
fn worn_vnum(lua: &Lua, cx: &mut Cx<'_>, first: &Value) -> mlua::Result<Option<Value>> {
    let Some(number) = lua.coerce_number(first.clone())? else {
        tracing::warn!("QUEST wrong set flag");
        return Ok(None);
    };
    // `(BYTE)` of the number, the low byte of `dword`, as `pc_send_block_mode` reads it.
    let cell = dword(number).to_le_bytes()[0];
    Ok(Some(
        cx.game
            .worn_vnum(cell)
            .map_or(Value::Nil, |vnum| Value::Number(f64::from(vnum))),
    ))
}

/// `pc_set_skill_level` (`G/questlua_pc.cpp:2112-2123`): `(DWORD)` and `(BYTE)` of
/// `lua_tonumber`, so a missing or non-number argument is 0. It pushes nothing.
fn set_skill_level(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue) -> mlua::Result<()> {
    let number = |at: usize| -> mlua::Result<u32> {
        match args.get(at) {
            Some(value) => Ok(lua.coerce_number(value.clone())?.map_or(0, dword)),
            None => Ok(0),
        }
    };
    let vnum = number(0)?;
    let level = number(1)?;
    cx.game.set_skill_level(vnum, level.to_le_bytes()[0]);
    Ok(())
}

/// `pc_remove_item` (`G/questlua_pc.cpp:801-860`), `pc.remove_item` and `pc.removeitem`. With
/// one argument, one of the vnum is taken; with two, the second, dworded, is the count, and a
/// count that is not a number is 0, which takes nothing. Any other number of arguments takes
/// nothing. The first argument, number-like, is the vnum, dworded; text is a locale name's
/// prefix, and a name no proto has, or anything else, takes nothing. It pushes nothing.
fn remove_item(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue) -> mlua::Result<()> {
    if !(1..=2).contains(&args.len()) {
        return Ok(());
    }
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let vnum = if let Some(vnum) = lua.coerce_number(first.clone())? {
        dword(vnum)
    } else if let Value::String(name) = &first {
        let name = host::c_string(lua, Value::String(name.clone()))?.unwrap_or_default();
        let Some(vnum) = cx.game.item_vnum(&name) else {
            tracing::warn!(
                name = %String::from_utf8_lossy(&name),
                "QUEST remove_item call error : wrong item name"
            );
            return Ok(());
        };
        vnum
    } else {
        tracing::warn!("QUEST remove_item wrong argument");
        return Ok(());
    };
    let count = match args.get(1) {
        None => 1,
        Some(count) => lua.coerce_number(count.clone())?.map_or(0, dword),
    };
    cx.game.remove_item(vnum, count);
    Ok(())
}

/// `pc_learn_grand_master_skill` (`questlua_pc.cpp:121-133`): a non-number pushes nothing; a
/// number pushes the climb. The count lives in the character's flags, so the second read in
/// one script sees the first.
fn learn_grand_master_skill(
    lua: &Lua,
    cx: &mut Cx<'_>,
    first: &Value,
) -> mlua::Result<Option<Value>> {
    let Some(vnum) = lua.coerce_number(first.clone())? else {
        return Ok(None);
    };
    let vnum = dword(vnum);
    let name = format!("training_grandmaster_skill.skill{vnum}");
    let total = cx
        .flag_map()
        .and_then(|flags| flags.get(name.as_bytes()).copied())
        .unwrap_or(0)
        + 1;
    if let Some(flags) = cx.flags() {
        flags.insert(name.into_bytes(), total);
    }
    let climbed = cx.game.learn_grand_master_skill(vnum, total);
    Ok(Some(Value::Boolean(climbed)))
}

/// 64-bit conversion, which is 0 where the conversion has no answer (out of range or NaN).
/// `(long)` of a dworded number: the bit-preserving `DWORD`->`long` conversion
/// `building_get_land_id`'s `(int)` casts do (`G/questlua_building.cpp:24`).
fn dword_long(number: f64) -> i32 {
    i32::from_ne_bytes(dword(number).to_ne_bytes())
}

fn dword(number: f64) -> u32 {
    let truncated = number.trunc();
    if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&truncated) {
        return 0;
    }
    let wide: i64 = format!("{truncated:.0}").parse().unwrap_or(0);
    let bytes = wide.to_le_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
/// `asctime(gmtime())` of a stamp (`_time_to_str`, `G/questlua_global.cpp:65-70`):
/// `Www Mmm DD HH:MM:SS YYYY\n`, the day blank-padded, in UTC. `None` (no argument or
/// a non-number one) is 0 (`(time_t)lua_tonumber`), and a fraction or a text stamp
/// truncates toward zero. The civil date is Howard Hinnant's days-from-epoch
/// algorithm, so no time crate is needed; the weekday of 1970-01-01 was a Thursday.
fn time_to_str(stamp: Option<f64>) -> Vec<u8> {
    const DAYS: [&[u8]; 7] = [b"Sun", b"Mon", b"Tue", b"Wed", b"Thu", b"Fri", b"Sat"];
    const MONTHS: [&[u8]; 12] = [
        b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov",
        b"Dec",
    ];
    // `(time_t)lua_tonumber`, truncated toward zero; out of range is 0, as the
    // 64-bit conversion answers it in `dword`.
    let seconds: i64 = match stamp {
        Some(stamp) => {
            let truncated = stamp.trunc();
            if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&truncated) {
                format!("{truncated:.0}").parse().unwrap_or(0)
            } else {
                0
            }
        }
        None => 0,
    };
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    // The days since 0000-03-01, from which the era breaks into years.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    let weekday = DAYS[usize::try_from(days.rem_euclid(7) + 4).unwrap_or(4) % 7];
    let month_name = MONTHS[usize::try_from(month - 1).unwrap_or(0).clamp(0, 11)];
    let (hour, minute, second) = (
        day_seconds / 3_600,
        day_seconds % 3_600 / 60,
        day_seconds % 60,
    );
    format!(
        "{} {} {:2} {:02}:{:02}:{:02} {}\n",
        String::from_utf8_lossy(weekday),
        String::from_utf8_lossy(month_name),
        day,
        hour,
        minute,
        second,
        year,
    )
    .into_bytes()
}

/// The [`crate::host::BRIDGED`] names (`questlua_global.cpp`, `questlua_game.cpp`,
/// `questlua_npc.cpp`).
/// What the text-output bridge did with a name.
enum TextOutcome {
    /// The name was none of its own; the caller keeps matching.
    Miss,
    /// The name wrote its text and returns this value.
    Hit(Option<Value>),
}

/// The bridged names that write text (`say`, the four chats, `raw_script`).
fn text_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
    last: Value,
) -> mlua::Result<TextOutcome> {
    let result = match name {
        b"say" => {
            let mut text = combine(lua, args)?;
            text.extend_from_slice(b"[ENTER]");
            cx.output.script.extend(text);
            None
        }
        b"chat" | b"cmdchat" | b"syschat" | b"notice" => {
            let kind = match name {
                b"chat" => ChatKind::Talking,
                b"cmdchat" => ChatKind::Command,
                b"syschat" => ChatKind::Info,
                _ => ChatKind::Notice,
            };
            let text = combine(lua, args)?;
            cx.game.chat(kind, &text);
            None
        }
        b"raw_script" => {
            if let Some(text) = host::c_string(lua, last)? {
                cx.output.script.extend(text);
            } else {
                tracing::warn!(
                    "QUEST wrong argument: questname: {}",
                    String::from_utf8_lossy(&cx.quest)
                );
            }
            None
        }
        _ => return Ok(TextOutcome::Miss),
    };
    Ok(TextOutcome::Hit(result))
}

/// The read-only character readers among the `pc.*` bridged names: gold, empire, HP,
/// SP, exp, race, name, VID, leadership, the four positions, the worn vnums, alignment,
/// and the guild triad. Each pushes one value and touches no state, so they sit apart
/// from the arms that write. Returns `None` for a name this table does not hold.
fn pc_readers(lua: &Lua, cx: &mut Cx<'_>, name: &[u8]) -> mlua::Result<Option<Value>> {
    let result = match name {
        // `pc_get_money` (`G/questlua_pc.cpp:1073-1077`) under the four names it is
        // registered as (`:3822-3823`, `:3830-3831`).
        b"pc.getmoney" | b"pc.get_money" | b"pc.getgold" | b"pc.get_gold" => {
            #[allow(clippy::cast_precision_loss)]
            // Legacy pushes `GetGold()` (`unsigned long long`, `:1076`) through
            // `lua_pushnumber` (`double`); gold above 2^53 rounds there too.
            let gold = cx.game.gold() as f64;
            Some(Value::Number(gold))
        }
        // `pc_get_empire` (`G/questlua_pc.cpp:1304-1308`) under both names (`:3904-3905`).
        b"pc.getempire" | b"pc.get_empire" => Some(Value::Number(f64::from(cx.game.empire()))),
        // `pc_get_hp` (`:975-979`) and `pc_get_max_hp` (`:969-973`), each under both
        // names (`:3813-3816`).
        b"pc.gethp" | b"pc.get_hp" => Some(Value::Number(f64::from(cx.game.hp()))),
        b"pc.getmaxhp" | b"pc.get_max_hp" => Some(Value::Number(f64::from(cx.game.max_hp()))),
        // `pc_get_sp` (`:927-931`) and `pc_get_max_sp` (`:921-925`), each under both
        // names (`:3817-3820`).
        b"pc.getsp" | b"pc.get_sp" => Some(Value::Number(f64::from(cx.game.sp()))),
        b"pc.getmaxsp" | b"pc.get_max_sp" => Some(Value::Number(f64::from(cx.game.max_sp()))),
        // `pc_get_exp` (`:897-901`) and `pc_get_next_exp` (`:891-895`).
        b"pc.get_exp" => Some(Value::Number(f64::from(cx.game.exp()))),
        b"pc.get_next_exp" => Some(Value::Number(f64::from(cx.game.next_exp()))),
        // `pc_get_race` (`:903-907`).
        b"pc.get_race" => Some(Value::Number(f64::from(cx.game.race()))),
        // `pc_get_name` (`:885-889`) under both names (`:3838-3839`).
        b"pc.getname" | b"pc.get_name" => {
            Some(Value::String(lua.create_string(cx.game.char_name())?))
        }
        // `pc_get_vid` (`:880-884`).
        b"pc.get_vid" => Some(Value::Number(f64::from(cx.game.char_vid()))),
        // `pc_get_player_id` (`:2608-2621`) and `pc_get_account_id` (`:2624-2638`).
        b"pc.get_player_id" => Some(Value::Number(f64::from(cx.game.player_id()))),
        b"pc.get_account_id" => Some(Value::Number(f64::from(cx.game.account_table_id()))),
        // `pc_get_leadership` (`:862-866`) under both names (`:3843-3844`).
        b"pc.getleadership" | b"pc.get_leadership" => {
            Some(Value::Number(f64::from(cx.game.leadership())))
        }
        // `pc_get_x`/`pc_get_y` (`:737-749`) and the local pair (`:751-775`), each under
        // both names (`:3850-3855`).
        b"pc.get_x" | b"pc.getx" => Some(Value::Number(f64::from(cx.game.position().0))),
        b"pc.get_y" | b"pc.gety" => Some(Value::Number(f64::from(cx.game.position().1))),
        b"pc.get_local_x" => Some(Value::Number(f64::from(cx.game.position().2))),
        b"pc.get_local_y" => Some(Value::Number(f64::from(cx.game.position().3))),
        // `pc_get_weapon` (`:1028-1038`) and `pc_get_armor` (`:1040-1050`): the worn vnum,
        // 0 when empty. (`pc_get_wear` sits in `pc_api`: a non-number argument pushes
        // nothing, which this table's `None` cannot say.)
        b"pc.getweapon" | b"pc.get_weapon" => {
            Some(Value::Number(f64::from(cx.game.worn_vnum(4).unwrap_or(0))))
        }
        b"pc.getarmor" | b"pc.get_armor" => {
            Some(Value::Number(f64::from(cx.game.worn_vnum(0).unwrap_or(0))))
        }
        // `pc_get_real_alignment` (`:1081-1085`) and `pc_get_alignment` (`:1087-1091`):
        // the real and the shown value, each /10. This build keeps one value (the shown
        // always follows the real), so one arm answers both.
        b"pc.get_real_alignment" | b"pc.get_alignment" => {
            Some(Value::Number(f64::from(cx.game.real_alignment() / 10)))
        }
        // `pc_hasguild` (`:325-330`) and `pc_isguildmaster` (`:339-350`), each under both
        // names (`:3895-3900`).
        b"pc.has_guild" | b"pc.hasguild" => Some(Value::Boolean(cx.game.guild_id() != 0)),
        b"pc.isguildmaster" | b"pc.is_guild_master" => {
            Some(Value::Boolean(cx.game.is_guild_master()))
        }
        // `pc_getguild` (`:332-337`) under both names (`:3897-3898`).
        b"pc.get_guild" | b"pc.getguild" => Some(Value::Number(f64::from(cx.game.guild_id()))),
        // `pc_is_gm` (`:1867-1871`) and `pc_get_gm_level` (`:1873-1877`): the gate at
        // `GM_HIGH_WIZARD` and the level itself.
        b"pc.is_gm" => Some(Value::Boolean(cx.game.gm_level() >= 3)),
        b"pc.get_gm_level" => Some(Value::Number(f64::from(cx.game.gm_level()))),
        // `pc_get_conqueror_level` (`G/questlua_pc.cpp:3632-3637`).
        b"pc.get_conqueror_level" => Some(Value::Number(f64::from(cx.game.conqueror_level()))),
        // `pc_get_conqueror_exp` (`G/questlua_pc.cpp:3617-3622`).
        b"pc.get_conqueror_exp" => Some(Value::Number(f64::from(cx.game.conqueror_exp()))),
        // `pc_get_conqueror_next_exp` (`:3624-3629`): `GetConquerorNextExp()`, the cost of
        // the conqueror level the game reports.
        b"pc.get_conqueror_next_exp" => {
            let next = common::levels::conqueror_next_exp(cx.game.conqueror_level());
            Some(Value::Number(f64::from(
                u32::try_from(next).unwrap_or(u32::MAX),
            )))
        }
        // `pc_is_mount` (`G/questlua_pc.cpp:1385-1390`): whether the mount vnum stands.
        b"pc.is_mount" => Some(Value::Boolean(cx.game.is_mount())),
        // `IsRiding` (`G/questlua_pc.cpp:2658-2672`): the horse ridden, or a mount.
        b"pc.is_riding" => Some(Value::Boolean(cx.game.horse().riding || cx.game.is_mount())),
        // `pc_get_channel_id` (`G/questlua_pc.cpp:2767-2772`): the Channel.
        b"pc.get_channel_id" => Some(Value::Number(f64::from(cx.game.channel_id()))),
        _ => return Ok(None),
    };
    Ok(result)
}

/// The `pc.*` bridged names: the read-only character readers (answered by
/// [`pc_readers`]), the level, the minutes played, the map, the quest flags, the
/// grand-master read, the skill level and the block mode. Returns `None` for a name
/// this table does not hold.
/// The quest-flag pair among [`pc_api`]'s names (`pc.getqf`, `pc.setqf`), split out so
/// neither function trips the line lint.
/// `pc_set_quest_flag` (`G/questlua_pc.cpp:1215-1229`): text or a number names a flag of
/// the quest that runs, and a number, rounded to even and cast to `int`, is its value
/// (`int(rint(...))`); 0 deletes the flag (`quest::PC::SetFlag`, `G/questpc.cpp:52-76`).
/// Anything else logs and changes nothing.
fn set_quest_flag(
    lua: &Lua,
    cx: &mut Cx<'_>,
    first: &Value,
    second: Value,
) -> mlua::Result<Option<Value>> {
    let flag = host::c_string(lua, first.clone())?;
    let value = lua.coerce_number(second)?;
    let (Some(flag), Some(value)) = (flag, value) else {
        tracing::warn!("QUEST wrong set flag");
        return Ok(None);
    };
    let value = host::c_int(lua, Value::Number(value.round_ties_even()))?;
    let name = [&cx.quest[..], b".", &flag].concat();
    if let Some(flags) = cx.flags() {
        if value == 0 {
            flags.remove(&name);
        } else {
            flags.insert(name, value);
        }
    }
    Ok(None)
}

fn pc_flags(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    first: &Value,
    last: &Value,
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let result: Option<Value> = match name {
        b"pc.getqf" => {
            // `pc_get_quest_flag` (`G/questlua_pc.cpp:1180-1197`): the last argument, text
            // or a number, names a flag of the quest that runs (`GetCurrentQuestName() + "."
            // + sz`), and a flag the character never set reads 0 (`quest::PC::GetFlag`,
            // `G/questpc.cpp:92-103`). Anything else logs and pushes nothing.
            let Some(flag) = host::c_string(lua, last.clone())? else {
                tracing::warn!("QUEST wrong get flag");
                return Ok(Some(MultiValue::new()));
            };
            let name = [&cx.quest[..], b".", &flag].concat();
            let value = cx
                .flag_map()
                .and_then(|flags| flags.get(&name).copied())
                .unwrap_or(0);
            Some(Value::Number(f64::from(value)))
        }
        b"pc.setqf" => set_quest_flag(lua, cx, first, args.get(1).cloned().unwrap_or(Value::Nil))?,
        b"pc.getf" => {
            // `pc_get_another_quest_flag` (`G/questlua_pc.cpp:1141-1165`): the first two
            // arguments name a flag of another quest (`sz.sz2`); text or a number names
            // each, as `pc.getqf` reads its one. An unset flag reads 0. Anything else
            // logs and pushes nothing.
            let second = args.get(1).cloned().unwrap_or(Value::Nil);
            let (Some(quest), Some(flag)) = (
                host::c_string(lua, first.clone())?,
                host::c_string(lua, second)?,
            ) else {
                tracing::warn!("QUEST wrong get flag");
                return Ok(Some(MultiValue::new()));
            };
            let name = [quest, b".".to_vec(), flag].concat();
            let value = cx
                .flag_map()
                .and_then(|flags| flags.get(&name).copied())
                .unwrap_or(0);
            Some(Value::Number(f64::from(value)))
        }
        b"pc.setf" => {
            // `pc_set_another_quest_flag` (`G/questlua_pc.cpp:1123-1139`): the first two
            // arguments name a flag of another quest, the third is its value; text or a
            // number names each, and the value rounds as `pc.setqf`'s does. Anything
            // else logs and changes nothing.
            let second = args.get(1).cloned().unwrap_or(Value::Nil);
            let third = args.get(2).cloned().unwrap_or(Value::Nil);
            let (Some(quest), Some(flag)) = (
                host::c_string(lua, first.clone())?,
                host::c_string(lua, second)?,
            ) else {
                tracing::warn!("QUEST wrong set flag");
                return Ok(Some(MultiValue::new()));
            };
            let Some(number) = lua.coerce_number(third)? else {
                tracing::warn!("QUEST wrong set flag");
                return Ok(Some(MultiValue::new()));
            };
            let value = host::c_int(lua, Value::Number(number.round_ties_even()))?;
            let name = [quest, b".".to_vec(), flag].concat();
            if let Some(flags) = cx.flags() {
                if value == 0 {
                    flags.remove(&name);
                } else {
                    flags.insert(name, value);
                }
            }
            None
        }
        b"pc.delqf" => {
            // `pc_del_quest_flag` (`G/questlua_pc.cpp:1231-1243`): text or a number names
            // a flag of the quest that runs, and it is deleted whether or not it was
            // set. Anything else logs and changes nothing.
            let Some(flag) = host::c_string(lua, first.clone())? else {
                tracing::warn!("argument error");
                return Ok(Some(MultiValue::new()));
            };
            let name = [&cx.quest[..], b".", &flag].concat();
            if let Some(flags) = cx.flags() {
                flags.remove(&name);
            }
            None
        }
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// The `pc.*` window calls: `hide_costume`, the sash and changelook opens, and
/// `get_wear`. Returns `None` for a name this table does not hold.
fn pc_windows(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
    first: &Value,
) -> mlua::Result<Option<MultiValue>> {
    let result: Option<Value> = match name {
        // `pc_hide_costume` (see `hide_costume`).
        b"pc.hide_costume" => {
            hide_costume(lua, cx, args)?;
            None
        }
        // `pc_open_sash` (see `open_sash`).
        b"pc.open_sash" => {
            open_sash(cx, args);
            None
        }
        // `pc_open_changelook` (see `open_changelook`).
        b"pc.open_changelook" => {
            open_changelook(cx);
            None
        }
        // `pc_mining` (`:1881-1887`): no argument is read; the NPC is the script's own.
        b"pc.mining" => {
            cx.game.mining(cx.npc_vid);
            None
        }
        // `pc_get_wear` (`:1052-1071`): the vnum worn in `(BYTE)` of the first argument,
        // nil when the cell is out of range or empty. A non-number argument logs and
        // pushes nothing.
        b"pc.get_wear" => worn_vnum(lua, cx, first)?,
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

fn pc_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let first = &args.front().cloned().unwrap_or(Value::Nil);
    let last = &args.back().cloned().unwrap_or(Value::Nil);
    if let Some(read) = pc_readers(lua, cx, name)? {
        return Ok(Some(MultiValue::from_iter([read])));
    }
    if let Some(window) = pc_windows(lua, cx, name, args, first)? {
        return Ok(Some(window));
    }
    let number = |value: u32| Some(Value::Number(f64::from(value)));
    let result = match name {
        b"pc.get_level" => number(u32::from(cx.game.level())),
        // `pc_get_job` (`G/questlua_pc.cpp:915-919`).
        b"pc.get_job" => number(u32::from(cx.game.job())),
        // `pc_get_playtime` (`G/questlua_pc.cpp:874-878`) under both names it is registered as
        // (`:3841-3842`).
        b"pc.getplaytime" | b"pc.get_playtime" => {
            Some(Value::Number(f64::from(cx.game.playtime())))
        }
        // `pc_change_alignment` (`G/questlua_pc.cpp:1093-1100`) under both names it is
        // registered as (`:3834`, `:3837`).
        b"pc.changealignment" | b"pc.change_alignment" => {
            change_alignment(lua, cx, first)?;
            None
        }
        // `pc_change_money` under its four names (see `change_gold`).
        b"pc.changegold" | b"pc.changemoney" | b"pc.change_gold" | b"pc.change_money" => {
            change_gold(lua, cx, last)?;
            None
        }
        b"pc.get_map_index" => Some(Value::Number(f64::from(cx.game.map_index()))),
        // The give, count and refine arms (see `pc_give`).
        b"pc.give_item2"
        | b"pc.give_item2_select"
        | b"pc.ore_refine"
        | b"pc.diamond_refine"
        | b"pc.count_item"
        | b"pc.countitem"
        | b"pc.remove_item"
        | b"pc.removeitem" => return pc_give(lua, cx, name, args),
        b"pc.get_war_map" => Some(Value::Number(f64::from(cx.game.war_map()))),
        // `pc_get_skillgroup` (`G/questlua_pc.cpp:1339-1343`).
        b"pc.get_skill_group" => number(u32::from(cx.game.skill_group())),
        // `pc_warp` (see `pc_warp`).
        b"pc.warp" => Some(Value::Boolean(pc_warp(lua, cx, args)?)),
        // `pc_warp_to_guild_war_observer_position` (see `war_observe`).
        b"pc.warp_to_guild_war_observer_position" => {
            war_observe(lua, cx, args)?;
            None
        }
        // `pc_enough_inventory` (`:478-499`): a vnum that is not number-like, or names no
        // proto, answers false; otherwise whether a fresh base cell below the unlocked
        // count is empty.
        b"pc.enough_inventory" => {
            let enough = match lua.coerce_number(first.clone())? {
                Some(vnum) => cx.game.has_empty_inventory(dword(vnum)),
                None => false,
            };
            Some(Value::Boolean(enough))
        }
        b"pc.set_skill_level" => {
            set_skill_level(lua, cx, args)?;
            None
        }
        b"pc.getqf" | b"pc.setqf" | b"pc.getf" | b"pc.setf" | b"pc.delqf" => {
            return pc_flags(lua, cx, name, first, last, args);
        }
        b"pc.learn_grand_master_skill" => {
            let Some(climbed) = learn_grand_master_skill(lua, cx, first)? else {
                return Ok(Some(MultiValue::new()));
            };
            Some(climbed)
        }
        // `pc_is_skill_book_no_delay` (`G/questlua_pc.cpp:113-119`) and
        // `pc_remove_skill_book_no_delay` (`:106-111`).
        b"pc.is_skill_book_no_delay" => Some(Value::Boolean(cx.game.has_skill_book_no_delay())),
        b"pc.remove_skill_book_no_delay" => {
            cx.game.remove_skill_book_no_delay();
            None
        }
        b"pc.get_skill_level" => return get_skill_level(lua, cx, first),
        b"pc.send_block_mode" => {
            send_block_mode(lua, cx, first)?;
            None
        }
        // `pc_can_warp` (`G/questlua_pc.cpp:2723-2737`).
        b"pc.can_warp" => Some(Value::Boolean(cx.game.can_warp())),
        // `pc_set_conqueror` (`G/questlua_pc.cpp:3639-3644`): pushes nothing.
        b"pc.set_conqueror" => {
            cx.game.set_conqueror();
            None
        }
        // `pc_destroy_guild` (`:352-370`) and `pc_remove_from_guild` (`:372-381`) push
        // nothing.
        b"pc.destroy_guild" => {
            cx.game.destroy_guild();
            None
        }
        b"pc.remove_from_guild" => {
            cx.game.remove_from_guild();
            None
        }
        // The polymorph, wedding, empire and Okey readers (see `pc_state`).
        b"pc.is_polymorphed"
        | b"pc.is_engaged"
        | b"pc.is_married"
        | b"pc.change_empire"
        | b"pc.get_change_empire_count"
        | b"pc.get_okay_global_rank"
        | b"pc.get_okay_rund_rank" => return pc_state(lua, cx, name, first),
        // The tail readers and writers (see `pc_tail`), the killee's pair among them.
        b"pc.get_empty_inventory_count"
        | b"pc.is_trade0"
        | b"pc.is_busy0"
        | b"pc.give_exp2"
        | b"pc.set_skill_group"
        | b"pc.warp_local"
        | b"pc.select"
        | b"pc.get_killee_drop_pct" => return pc_tail(lua, cx, name, args, first),
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// The `pc.*` give, count and refine arms: `pc.give_item2` (see `give_item2`),
/// `pc.give_item2_select` (see `give_item2_select`), the ore and diamond refines (see
/// `ore_refine`), and the count and remove pair (see `count_item` and `remove_item`).
/// Split out so `pc_api` stays under the line lint.
fn pc_give(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let number = |value: u32| Some(Value::Number(f64::from(value)));
    let result = match name {
        b"pc.give_item2" => number(give_item2(lua, cx, args)?),
        b"pc.give_item2_select" => {
            give_item2_select(lua, cx, args)?;
            None
        }
        b"pc.ore_refine" | b"pc.diamond_refine" => {
            Some(Value::Boolean(ore_refine(lua, cx, name, args)?))
        }
        b"pc.count_item" | b"pc.countitem" => {
            Some(Value::Number(f64::from(count_item(lua, cx, args)?)))
        }
        b"pc.remove_item" | b"pc.removeitem" => {
            remove_item(lua, cx, args)?;
            None
        }
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}
/// `pc_send_block_mode` (`G/questlua_pc.cpp:2190-2197`): the number becomes the
/// character's own block mode; a non-number is zero (`(BYTE)lua_tonumber`), and only
/// the low byte survives. Split out so `pc_api` stays under the line lint.
fn send_block_mode(lua: &Lua, cx: &mut Cx<'_>, first: &Value) -> mlua::Result<()> {
    let mode = match lua.coerce_number(first.clone())? {
        Some(number) => {
            let bytes = dword(number).to_le_bytes();
            bytes[0]
        }
        None => 0,
    };
    cx.game.send_block_mode(mode);
    Ok(())
}

/// The polymorph, wedding and empire bridge: `pc_is_polymorphed`
/// (`G/questlua_pc.cpp:1361-1366`), `pc_is_engaged` (`:1846-1851`), `pc_is_married`
/// (`:1853-1858`), `pc_change_empire` (`:2199-2213`) and `pc_get_change_empire_count`
/// (`:2215-2221`). Split out so `pc_api` stays under the line lint.
fn pc_state(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    first: &Value,
) -> mlua::Result<Option<MultiValue>> {
    let result = match name {
        // `pc_is_polymorphed` (`G/questlua_pc.cpp:1361-1366`).
        b"pc.is_polymorphed" => Some(Value::Boolean(cx.game.is_polymorphed())),
        // `pc_get_okay_global_rank` (`G/questlua_pc.cpp:80-91`) and
        // `pc_get_okay_rund_rank` (`:93-104`): the rank text, empty with no game.
        b"pc.get_okay_global_rank" => Some(Value::String(
            lua.create_string(cx.game.okay_global_rank())?,
        )),
        b"pc.get_okay_rund_rank" => {
            Some(Value::String(lua.create_string(cx.game.okay_round_rank())?))
        }
        // `pc_is_engaged` (`:1846-1851`) and `pc_is_married` (`:1853-1858`).
        b"pc.is_engaged" => Some(Value::Boolean(cx.game.is_engaged())),
        b"pc.is_married" => Some(Value::Boolean(cx.game.is_married())),
        // `pc_change_empire` (`G/questlua_pc.cpp:2199-2213`): `(BYTE)lua_tonumber` of the
        // first argument is the empire, and `ChangeEmpire` answers 0, 1, 2, 3, 4 or 999.
        // The party chat after it is legacy's own second push: `ChangeEmpire` returned
        // 4 already, the chat warns, and the pushed 4 stays the pushed 4 — so the
        // bridge pushes the world's answer once, and the world chats the line.
        b"pc.change_empire" => {
            let empire = match lua.coerce_number(first.clone())? {
                Some(number) => dword(number).to_le_bytes()[0],
                None => 0,
            };
            Some(Value::Number(f64::from(cx.game.change_empire(empire))))
        }
        // `pc_get_change_empire_count` (`:2215-2221`).
        b"pc.get_change_empire_count" => {
            Some(Value::Number(f64::from(cx.game.change_empire_count())))
        }
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// The `pc.*` tail readers and writers: the empty count, the trade and busy gates, the
/// exp grant, the skill group, the local warp, the VID switch and the killee's drop pair
/// (`G/questlua_pc.cpp`). Split out so `pc_api` stays under the line lint.
fn pc_tail(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
    first: &Value,
) -> mlua::Result<Option<MultiValue>> {
    let result: Option<Value> = match name {
        // `pc_get_empty_inventory_count` (`:2576-2590`).
        b"pc.get_empty_inventory_count" => {
            Some(Value::Number(f64::from(cx.game.empty_inventory_count())))
        }
        // `pc_is_trade0` (`:3379-3384`) and `pc_is_busy0` (`:3386-3392`).
        b"pc.is_trade0" => Some(Value::Boolean(cx.game.is_trade0())),
        b"pc.is_busy0" => Some(Value::Boolean(cx.game.is_busy0())),
        // `pc_give_exp2` (`:1245-1261`): `(DWORD)rint` of a number is the grant; a
        // non-number runs nothing. Pushes nothing.
        b"pc.give_exp2" => {
            if let Some(number) = lua.coerce_number(first.clone())? {
                cx.game.give_exp2(dword(number.round_ties_even()));
            }
            None
        }
        // `pc_set_skillgroup` (`:1345-1358`): a number sets; anything else only logs.
        // Pushes nothing.
        b"pc.set_skill_group" => {
            if let Some(number) = lua.coerce_number(first.clone())? {
                let group = dword(number).to_le_bytes()[0];
                cx.game.set_skill_group(group);
            } else {
                tracing::warn!("QUEST wrong skillgroup number");
            }
            None
        }
        // `pc_warp_local` (`:242-316`): three numbers warp to the region's base plus
        // the pair; anything else only logs. Pushes nothing.
        b"pc.warp_local" => {
            let index = args.front().cloned().unwrap_or(Value::Nil);
            let x = args.get(1).cloned().unwrap_or(Value::Nil);
            let y = args.get(2).cloned().unwrap_or(Value::Nil);
            let numbers = (
                lua.coerce_number(index.clone())?,
                lua.coerce_number(x.clone())?,
                lua.coerce_number(y.clone())?,
            );
            match numbers {
                (Some(_), Some(_), Some(_)) => {
                    let index = host::c_int(lua, index)?;
                    let x = host::c_int(lua, x)?;
                    let y = host::c_int(lua, y)?;
                    cx.game.warp_local(index, x, y);
                }
                (None, _, _) => tracing::warn!("QUEST no map index argument"),
                _ => tracing::warn!("QUEST no coodinate argument"),
            }
            None
        }
        // `pc_select_vid` (`:1814-1836`): `lua_tonumber` of the first argument is the
        // VID; the old VID is pushed, or 0 with none under it.
        b"pc.select" => {
            let vid = dword(lua.coerce_number(first.clone())?.unwrap_or(0.0));
            Some(Value::Number(f64::from(cx.game.select_pc(vid))))
        }
        // `pc_get_killee_drop_pct` (`:2962-2980`): two pushes, so its own return.
        b"pc.get_killee_drop_pct" => {
            let (delta, range) = killee_drop_pct(cx);
            return Ok(Some(MultiValue::from_iter([delta, range])));
        }
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// `pc_get_killee_drop_pct` (`G/questlua_pc.cpp:2962-2980`): the killee's drop pair for
/// the quest NPC (`cx.npc_vid`, which the kill's `SetQuestNPCID` recorded). Split out
/// so `pc_tail` stays under the line lint.
fn killee_drop_pct(cx: &mut Cx<'_>) -> (Value, Value) {
    let (delta, range) = cx.game.killee_drop_pct(cx.npc_vid);
    (
        Value::Number(f64::from(delta)),
        Value::Number(f64::from(range)),
    )
}

/// `pc_get_skill_level` (`G/questlua_pc.cpp:1734-1749`): a non-number logs and answers 0;
/// a number answers the level. Split out so `pc_api` stays under the line lint.
fn get_skill_level(lua: &Lua, cx: &mut Cx<'_>, first: &Value) -> mlua::Result<Option<MultiValue>> {
    let Some(vnum) = lua.coerce_number(first.clone())? else {
        tracing::warn!("QUEST invalid argument");
        return Ok(Some(MultiValue::from_iter([Value::Number(0.0)])));
    };
    Ok(Some(MultiValue::from_iter([Value::Number(f64::from(
        cx.game.skill_level(dword(vnum)),
    ))])))
}

/// The `guild.*` bridged names (230s): the core readers and writers, answered by the world's
/// guilds now, while the script that asked still waits. Each argument dword-decides as
/// legacy's `lua_tonumber`/`lua_tostring` do: a number-like one arrives dworded, a
/// text-only one arrives raw, and anything else arrives as the empty text. Returns `None`
/// for a name this table does not hold.
fn guild_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let Some(member) = name.strip_prefix(b"guild.") else {
        return Ok(None);
    };
    let bridged = matches!(
        member,
        b"get_rank"
            | b"get_ladder_point"
            | b"high_ranking_string"
            | b"around_ranking_string"
            | b"name"
            | b"level"
            | b"war_enter"
            | b"get_any_war"
            | b"get_reserve_war_table"
            | b"war_bet"
            | b"is_bet"
            | b"get_warp_war_list"
            | b"get_name"
            | b"get_member_count"
            | b"change_master"
            | b"change_master_with_limit"
            | b"get_id0"
            | b"get_sp0"
            | b"get_maxsp0"
            | b"get_money0"
            | b"get_max_member0"
            | b"get_total_member_level0"
            | b"has_land0"
            | b"get_win_count0"
            | b"get_draw_count0"
            | b"get_loss_count0"
            | b"add_comment0"
            | b"set_ladder_point0"
            | b"get_skill_level0"
            | b"set_skill_level0"
            | b"get_skill_point0"
            | b"set_skill_point0"
            | b"get_exp_level0"
            | b"offer_exp0"
            | b"give_exp0"
    );
    if !bridged {
        return Ok(None);
    }
    let mut judged = Vec::with_capacity(args.len());
    for arg in args {
        if let Some(number) = lua.coerce_number(arg.clone())? {
            judged.push(GuildArg::Number(dword(number)));
        } else {
            judged.push(GuildArg::Text(
                host::c_string(lua, arg.clone())?.unwrap_or_default(),
            ));
        }
    }
    let answer = cx.game.guild_call(member, &judged);
    let pushed = match answer {
        GuildAnswer::None => MultiValue::new(),
        GuildAnswer::Number(value) => MultiValue::from_iter([Value::Number(value)]),
        GuildAnswer::Boolean(value) => MultiValue::from_iter([Value::Boolean(value)]),
        GuildAnswer::Text(text) => {
            MultiValue::from_iter([Value::String(lua.create_string(&text)?)])
        }
        GuildAnswer::Table(rows) => {
            let table = lua.create_table()?;
            for (row, cells) in rows.iter().enumerate() {
                let inner = lua.create_table()?;
                for (cell, value) in cells.iter().enumerate() {
                    inner.set(cell + 1, *value)?;
                }
                table.set(row + 1, inner)?;
            }
            MultiValue::from_iter([Value::Table(table)])
        }
    };
    Ok(Some(pushed))
}

/// The `party.*` bridged calls: the two readers, the leader PID, and the flag
/// pair. Returns `None` for a name this table does not hold.
fn party_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let result: Option<Value> = match name {
        // `party_is_party` (`G/questlua_party.cpp:236-241`).
        b"party.is_party" => Some(Value::Boolean(cx.game.is_party())),
        // `party_is_leader` (`G/questlua_party.cpp:224-234`).
        b"party.is_leader" => Some(Value::Boolean(cx.game.is_party_leader())),
        // `party_get_leader_pid` (`G/questlua_party.cpp:243-255`): -1 with none.
        b"party.get_leader_pid" => Some(Value::Number(f64::from(cx.game.party_leader_pid()))),
        // `party_get_near_count` (`G/questlua_party.cpp:195-204`): the cached near
        // count, 0 with no party.
        b"party.get_near_count" => Some(Value::Number(f64::from(cx.game.party_near_count()))),
        // `party_get_member_pids` (`G/questlua_party.cpp:424-442`) pushes one value
        // per PID, so it returns early with its own arity.
        b"party.get_member_pids" => {
            return Ok(Some(
                cx.game
                    .party_member_pids()
                    .into_iter()
                    .map(|pid| Value::Number(f64::from(pid)))
                    .collect(),
            ));
        }
        // `party_get_flag` (`G/questlua_party.cpp:315-325`): 0 with no party or
        // a non-string name.
        b"party.getf" => {
            let value = match host::c_string(lua, first)? {
                Some(flag) => cx.game.party_flag(&flag),
                None => 0,
            };
            Some(Value::Number(f64::from(value)))
        }
        // `party_set_flag` (`G/questlua_party.cpp:305-313`): a string name and
        // a number value set; anything else is quiet. Pushes nothing.
        b"party.setf" => {
            let second = args.get(1).cloned().unwrap_or(Value::Nil);
            if let (Some(flag), Some(value)) =
                (host::c_string(lua, first)?, lua.coerce_number(second)?)
            {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                // Legacy casts `(int)lua_tonumber`.
                let value = value as i32;
                cx.game.set_party_flag(flag, value);
            }
            None
        }
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// The `horse.*` calls (`G/questlua_horse.cpp:24-291`), and `pc.get_horse_level`,
/// `pc.is_horse_alive` and `pc.revive_horse` (`G/questlua_pc.cpp:1491-1531`). The pools and
/// their percentages read 0 without a horse; the summon, unsummon, revive, ride and naming
/// run in the world when the step ends. Returns `None` for a name this table does not hold.
fn horse_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let horse = cx.game.horse();
    let number = |value: i32| Some(Value::Number(f64::from(value)));
    // `MINMAX(0, x * 100 / max, 100)`, pushed only with a horse.
    let owned = |value: i16, max: i16, pct: bool| {
        if horse.level == 0 {
            0
        } else if pct {
            (i32::from(value) * 100 / i32::from(max).max(1)).clamp(0, 100)
        } else {
            i32::from(value)
        }
    };
    let result: Option<Value> = match name {
        b"horse.is_riding" => number(i32::from(horse.riding)),
        b"horse.get_level" | b"pc.get_horse_level" => number(i32::from(horse.level)),
        b"horse.get_health" => number(owned(horse.health, horse.max_health, false)),
        b"horse.get_health_pct" => number(owned(horse.health, horse.max_health, true)),
        b"horse.get_stamina" => number(owned(horse.stamina, horse.max_stamina, false)),
        b"horse.get_stamina_pct" => number(owned(horse.stamina, horse.max_stamina, true)),
        b"horse.get_grade" => number(i32::from(horse.grade())),
        b"horse.is_dead" => Some(Value::Boolean(horse.health <= 0)),
        b"pc.is_horse_alive" => Some(Value::Boolean(horse.level > 0 && horse.health > 0)),
        // `horse_set_level`: only a number (or a numeric string) sets, `(int)` then clamped.
        b"horse.set_level" => {
            let first = args.front().cloned().unwrap_or(Value::Nil);
            if lua.coerce_number(first.clone())?.is_some() {
                let level = host::c_int(lua, first)?.clamp(0, 30);
                cx.game.set_horse_level(level);
            }
            None
        }
        b"horse.advance" => {
            if horse.level < 30 {
                cx.game.set_horse_level(i32::from(horse.level) + 1);
            }
            None
        }
        b"horse.feed" => {
            if horse.level > 0 && horse.health > 0 {
                cx.game.feed_horse();
            }
            None
        }
        b"horse.is_summon" => Some(Value::Boolean(horse.summoned != 0)),
        b"horse.is_mine" => Some(Value::Boolean(
            horse.summoned != 0 && horse.summoned == cx.npc_vid,
        )),
        // `horse_summon`: a boolean first argument, a number (or numeric string) second as
        // `(DWORD)`; the name third is never read.
        b"horse.summon" => {
            let from_far = matches!(args.front(), Some(Value::Boolean(true)));
            let second = args.get(1).cloned().unwrap_or(Value::Nil);
            let vnum = host::c_int(lua, second)?;
            cx.game
                .summon_horse(from_far, u32::from_ne_bytes(vnum.to_ne_bytes()));
            None
        }
        b"horse.unsummon" => {
            cx.game.unsummon_horse();
            None
        }
        b"horse.ride" => {
            // The OX map is 113 (`G/questlua_horse.cpp:59`); no map is an arena map
            // (`sys.event.arena`).
            if !cx.game.is_mount() && cx.game.map_index() != 113 {
                cx.game.ride_horse();
            }
            None
        }
        b"horse.unride" => {
            cx.game.unride_horse();
            None
        }
        b"horse.revive" => {
            if horse.level > 0 && horse.health <= 0 {
                cx.game.revive_horse();
            }
            None
        }
        b"pc.revive_horse" => {
            cx.game.revive_horse();
            None
        }
        // `horse_set_name`: nothing is pushed for a non-string last argument
        // (`lua_isstring`, which a number fails); else the world names and answers 2,
        // 1 or 0. `horse_get_name` pushes the manager's name, empty with none.
        b"horse.set_name" => {
            let last = args.back().cloned().unwrap_or(Value::Nil);
            if !matches!(last, Value::String(_)) {
                return Ok(Some(MultiValue::new()));
            }
            let text = host::c_string(lua, last)?.unwrap_or_default();
            number(i32::from(cx.game.set_horse_name(text)))
        }
        b"horse.get_name" => Some(Value::String(lua.create_string(cx.game.horse_name())?)),
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// `pc.mount`, `pc.mount_bonus` and `pc.unmount` (`G/questlua_pc.cpp:1392-1489`), run in the
/// world when the step ends. Returns `None` for a name this table does not hold.
fn mount_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    match name {
        // `pc_mount` (`G/questlua_pc.cpp:1392-1458`): nothing without a number first; the
        // seconds are the second number, 60 without one or below 0.
        b"pc.mount" => {
            let first = args.front().cloned().unwrap_or(Value::Nil);
            if lua.coerce_number(first.clone())?.is_none() {
                return Ok(Some(MultiValue::new()));
            }
            let second = args.get(1).cloned().unwrap_or(Value::Nil);
            let seconds = if lua.coerce_number(second.clone())?.is_some() {
                host::c_int(lua, second)?
            } else {
                60
            };
            let vnum = host::c_int(lua, first)?;
            cx.game.mount(
                u32::from_ne_bytes(vnum.to_ne_bytes()),
                if seconds < 0 { 60 } else { seconds },
            );
            Ok(Some(MultiValue::new()))
        }
        b"pc.unmount" => {
            cx.game.unmount();
            Ok(Some(MultiValue::new()))
        }
        // `pc_mount_bonus` (`G/questlua_pc.cpp:1461-1479`): the apply type as `(BYTE)`, the
        // value and the seconds as `(long)`.
        b"pc.mount_bonus" => {
            let apply = host::c_int(lua, args.front().cloned().unwrap_or(Value::Nil))?;
            let value = host::c_int(lua, args.get(1).cloned().unwrap_or(Value::Nil))?;
            let seconds = host::c_int(lua, args.get(2).cloned().unwrap_or(Value::Nil))?;
            cx.game.mount_bonus(apply.to_le_bytes()[0], value, seconds);
            Ok(Some(MultiValue::new()))
        }
        _ => Ok(None),
    }
}

/// The regist's three flags, by the names legacy writes (`G/char_item.cpp:10601-10605`).
const ATTR67_FLAG_SUCCESS: &[u8] = b"attr67add_item.success";
/// The wait's end, which the quest reads but this family never writes.
const ATTR67_FLAG_WAIT_TIME: &[u8] = b"attr67add_item.wait_time";
/// The rare-add control.
const ATTR67_FLAG_ADD: &[u8] = b"attr67add_item.add";
/// The three flags `attr67add.collect` resets (`G/questlua_attr67add.cpp:61-63`): 0 deletes
/// each, as `SetFlag` deletes a 0.
const ATTR67_FLAGS: [&[u8]; 3] = [ATTR67_FLAG_SUCCESS, ATTR67_FLAG_WAIT_TIME, ATTR67_FLAG_ADD];
/// `attr67add.collect` with no room (`G/questlua_attr67add.cpp:66`).
const ATTR67_NO_ROOM: &[u8] = b"Not enough inventory space.";

/// The `attr67add.*` calls (`__ATTR_6TH_7TH__`, `G/questlua_attr67add.cpp:28-137`): the
/// window's readers and its collect. Returns `None` for a name this table does not hold.
/// It cannot fail, but the [`Family`] table calls through `Result`, so it keeps the wrap.
#[allow(clippy::unnecessary_wraps)]
fn attr67_api(
    _lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    _args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let one = |value: Value| MultiValue::from_iter([value]);
    match name {
        // `attr67add_holding` (`G/questlua_attr67add.cpp:28-42`): whether the window
        // holds an item.
        b"attr67add.holding" => Ok(Some(one(Value::Boolean(cx.game.attr67_vnum() != 0)))),
        b"attr67add.success" => {
            // `attr67add_success` (`G/questlua_attr67add.cpp:75-99`): false with no
            // window item; else whether `attr67add_item.success` is above 0, adding
            // the rare attribute first when `attr67add_item.add` is 0 or below.
            if cx.game.attr67_vnum() == 0 {
                return Ok(Some(one(Value::Boolean(false))));
            }
            let held = cx
                .flag_map()
                .and_then(|flags| flags.get(ATTR67_FLAG_SUCCESS))
                .copied()
                .unwrap_or(0);
            if held <= 0 {
                return Ok(Some(one(Value::Boolean(false))));
            }
            let added = cx
                .flag_map()
                .and_then(|flags| flags.get(ATTR67_FLAG_ADD))
                .copied()
                .unwrap_or(0);
            if added <= 0 {
                cx.game.attr67_add_rare();
                if let Some(flags) = cx.flags() {
                    flags.insert(ATTR67_FLAG_ADD.to_vec(), 1);
                }
            }
            Ok(Some(one(Value::Boolean(true))))
        }
        b"attr67add.collect" => {
            // `attr67add_collect` (`G/questlua_attr67add.cpp:46-71`): nothing with no
            // window item; else the move home, and the three flags reset. `Not enough
            // inventory space.` with no room. It pushes nothing.
            if cx.game.attr67_vnum() == 0 {
                return Ok(Some(MultiValue::new()));
            }
            if cx.game.attr67_collect() {
                if let Some(flags) = cx.flags() {
                    for flag in ATTR67_FLAGS {
                        flags.remove(flag);
                    }
                }
            } else {
                cx.game.chat(ChatKind::Info, ATTR67_NO_ROOM);
            }
            Ok(Some(MultiValue::new()))
        }
        // `attr67add_item_vnum` (`G/questlua_attr67add.cpp:103-117`): the window's
        // vnum, 0 with none.
        b"attr67add.item_vnum" => Ok(Some(one(Value::Number(f64::from(cx.game.attr67_vnum()))))),
        // `attr67add_window_type` (`:121-127`): `ATTR67_ADD`.
        b"attr67add.window_type" => Ok(Some(one(Value::Number(f64::from(
            common::item_slots::EWindows::Attr67Add as u8,
        ))))),
        // `attr67add_window_cell` (`:129-137`): 0, the window's one cell.
        b"attr67add.window_cell" => Ok(Some(one(Value::Number(0.0)))),
        _ => Ok(None),
    }
}

/// The global rank's calls (`ENABLE_GLOBAL_RANK`) and the three `npc.is_*` reads its kill
/// script makes. Returns `None` for a name this table does not hold.
fn rank_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let boolean = |value: bool| Ok(Some(MultiValue::from_vec(vec![Value::Boolean(value)])));
    match name {
        // `pc_add_globalrank_point` (`G/questlua_pc.cpp:3723-3736`): nothing unless both are
        // numbers; the type as `(BYTE)` and the count as `(int)`.
        b"pc.global_rank_add_point" => {
            let first = args.front().cloned().unwrap_or(Value::Nil);
            let second = args.get(1).cloned().unwrap_or(Value::Nil);
            if lua.coerce_number(first.clone())?.is_none()
                || lua.coerce_number(second.clone())?.is_none()
            {
                return Ok(Some(MultiValue::new()));
            }
            let kind = host::c_int(lua, first)?.to_le_bytes()[0];
            let count = host::c_int(lua, second)?;
            cx.game.global_rank_add_point(kind, count);
            Ok(Some(MultiValue::new()))
        }
        b"pc.global_rank_get_reward" => {
            let won = cx.game.global_rank_get_reward();
            Ok(Some(MultiValue::from_vec(vec![Value::Number(f64::from(
                u8::from(won),
            ))])))
        }
        b"pc.in_dungeon" => boolean(cx.game.in_dungeon()),
        b"npc.is_pc" => boolean(cx.npc_vid != 0 && cx.game.npc_is_pc(cx.npc_vid)),
        // `npc_is_metin`, `npc_is_boss`: with no NPC the 0 they push is never returned, so
        // the script reads `nil`.
        b"npc.is_metin" | b"npc.is_boss" => {
            let read = (cx.npc_vid != 0)
                .then(|| cx.game.npc_stone_and_rank(cx.npc_vid))
                .flatten();
            let Some((stone, rank)) = read else {
                return Ok(Some(MultiValue::new()));
            };
            if name == b"npc.is_metin" {
                boolean(stone)
            } else {
                boolean(rank >= MOB_RANK_BOSS)
            }
        }
        _ => Ok(None),
    }
}

/// `MOB_RANK_BOSS` (`common/length.h:458-466`).
const MOB_RANK_BOSS: u8 = 4;

/// The `d.*` bridged calls (the jump, the map index, the flags, the notice, the warp
/// spot, the exit and the spawn) and the `ds.*` names, which read and grant the
/// dragon-soul qualification and open its windows. Returns `None` for a name this table
/// does not hold.
/// `dungeon_spawn_mob`: three numbers, the vnum as `(DWORD)`. Pushes nothing.
fn dungeon_spawn_mob(
    lua: &Lua,
    cx: &mut Cx<'_>,
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let vnum = args.front().cloned().unwrap_or(Value::Nil);
    let x = args.get(1).cloned().unwrap_or(Value::Nil);
    let y = args.get(2).cloned().unwrap_or(Value::Nil);
    let (Some(vnum), Some(_), Some(_)) = (
        lua.coerce_number(vnum.clone())?,
        lua.coerce_number(x.clone())?,
        lua.coerce_number(y.clone())?,
    ) else {
        return Ok(Some(MultiValue::new()));
    };
    cx.game
        .dungeon_spawn_mob(dword(vnum), host::c_int(lua, x)?, host::c_int(lua, y)?);
    Ok(Some(MultiValue::new()))
}

/// Three checked numbers, each as `(int)`: what `d.new_jump_party`, `d.new_jump`
/// and `d.set_warp_location` read (`G/questlua_dungeon.cpp:407-432`, `:318-348`,
/// `:1603-1622`). `None` unless all three are numbers.
fn dungeon_triple(lua: &Lua, args: &MultiValue) -> mlua::Result<Option<[i32; 3]>> {
    let numbers = [
        args.front().cloned().unwrap_or(Value::Nil),
        args.get(1).cloned().unwrap_or(Value::Nil),
        args.get(2).cloned().unwrap_or(Value::Nil),
    ];
    let mut checked = [0; 3];
    for (slot, value) in checked.iter_mut().zip(numbers) {
        if lua.coerce_number(value.clone())?.is_none() {
            return Ok(None);
        }
        *slot = host::c_int(lua, value)?;
    }
    Ok(Some(checked))
}

/// Two checked numbers, each as `(DWORD)`: what `d.set_damage_from_race` and
/// `d.set_damage_from_affect` read (`G/questlua_dungeon.cpp:902-938`). `None`
/// unless both are numbers.
fn dungeon_dword_pair(lua: &Lua, args: &MultiValue) -> mlua::Result<Option<(u32, u32)>> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let second = args.get(1).cloned().unwrap_or(Value::Nil);
    match (
        lua.coerce_number(first.clone())?,
        lua.coerce_number(second.clone())?,
    ) {
        (Some(first), Some(second)) => Ok(Some((dword(first), dword(second)))),
        _ => Ok(None),
    }
}

/// `dungeon_select` (`G/questlua_dungeon.cpp:1540-1563`) and `dungeon_find`
/// (`:1565-1585`): `(DWORD)lua_tonumber`, false with no number or index 0.
/// `d.select` names the dungeon for the `d.*` names while no character runs;
/// `d.find` only asks.
fn dungeon_select_find(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<MultiValue> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let index = match lua.coerce_number(first)? {
        Some(index) => dword(index),
        None => 0,
    };
    let lives = index != 0 && cx.game.dungeon_lives(index);
    if name == b"d.select" {
        cx.selected = lives.then_some(index);
        // `GetCurrentDungeon` (`G/questmanager.cpp:1252-1264`) ignores the
        // selection while a character runs; with none (PID 0, a server
        // timer's step) the session adopts it, or 0 when the select missed.
        if cx.current == 0 {
            let adopted = lives.then(|| i32::try_from(index).ok()).flatten();
            cx.game.dungeon_select(adopted);
        }
    }
    Ok(MultiValue::from_iter([Value::Boolean(lives)]))
}

fn dungeon_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    if matches!(name, b"d.select" | b"d.find") {
        return dungeon_select_find(lua, cx, name, args).map(Some);
    }
    match name {
        // `dungeon_new_jump_party` (`G/questlua_dungeon.cpp:407-432`): three numbers, the
        // map as `(long)`. Pushes nothing.
        b"d.new_jump_party" => {
            if let Some(checked) = dungeon_triple(lua, args)? {
                cx.game
                    .dungeon_new_jump_party(checked[0], checked[1], checked[2]);
            }
            Ok(Some(MultiValue::new()))
        }
        // `dungeon_new_jump` (`G/questlua_dungeon.cpp:318-348`): three numbers, the
        // map as `(long)`. Pushes nothing.
        b"d.new_jump" => {
            if let Some(checked) = dungeon_triple(lua, args)? {
                cx.game.dungeon_new_jump(checked[0], checked[1], checked[2]);
            }
            Ok(Some(MultiValue::new()))
        }
        // `dungeon_get_map_index` (`G/questlua_dungeon.cpp:184-201`).
        b"d.get_map_index" => {
            let index = cx.game.dungeon_map_index();
            Ok(Some(MultiValue::from_vec(vec![Value::Number(f64::from(
                index,
            ))])))
        }
        // `dungeon_set_flag` (`G/questlua_dungeon.cpp:106-130`): a string name and a
        // number value. Pushes nothing.
        b"d.setf" => {
            let name = args.front().cloned().unwrap_or(Value::Nil);
            let value = args.get(1).cloned().unwrap_or(Value::Nil);
            if let (Some(name), Some(_)) = (
                host::c_string(lua, name)?,
                lua.coerce_number(value.clone())?,
            ) {
                cx.game.dungeon_set_flag(name, host::c_int(lua, value)?);
            }
            Ok(Some(MultiValue::new()))
        }
        // `dungeon_get_flag` (`G/questlua_dungeon.cpp:131-153`).
        b"d.getf" => {
            let name = args.front().cloned().unwrap_or(Value::Nil);
            let flag = host::c_string(lua, name)?.map_or(0, |name| cx.game.dungeon_get_flag(&name));
            Ok(Some(MultiValue::from_vec(vec![Value::Number(f64::from(
                flag,
            ))])))
        }
        // `dungeon_get_flag_from_map_index` (`G/questlua_dungeon.cpp:155-182`): a
        // boolean false for a map index of 0, else the flag's number.
        b"d.getf_from_map_index" => {
            let name = args.front().cloned().unwrap_or(Value::Nil);
            let index = args.get(1).cloned().unwrap_or(Value::Nil);
            let index = match lua.coerce_number(index.clone())? {
                Some(index) => dword(index),
                None => 0,
            };
            if index == 0 {
                return Ok(Some(MultiValue::from_vec(vec![Value::Boolean(false)])));
            }
            let flag = host::c_string(lua, name)?
                .map_or(0, |name| cx.game.dungeon_flag_from_index(&name, index));
            Ok(Some(MultiValue::from_vec(vec![Value::Number(f64::from(
                flag,
            ))])))
        }
        // `dungeon_notice` (`G/questlua_dungeon.cpp:52-86`): a string. Pushes nothing.
        b"d.notice" => {
            if let Some(text) = host::c_string(lua, args.front().cloned().unwrap_or(Value::Nil))? {
                cx.game.dungeon_notice(text);
            }
            Ok(Some(MultiValue::new()))
        }
        // `dungeon_set_warp_location` (`G/questlua_dungeon.cpp:1603-1622`): three
        // numbers. Pushes nothing.
        b"d.set_warp_location" => {
            if let Some(checked) = dungeon_triple(lua, args)? {
                cx.game
                    .dungeon_set_warp_location(checked[0], checked[1], checked[2]);
            }
            Ok(Some(MultiValue::new()))
        }
        // `dungeon_exit_all` (`G/questlua_dungeon.cpp:1257-1266`). Pushes nothing.
        b"d.exit_all" => {
            cx.game.dungeon_exit_all();
            Ok(Some(MultiValue::new()))
        }
        // `dungeon_spawn_mob`: three numbers, the vnum as `(DWORD)`. Pushes nothing.
        b"d.spawn_mob" => dungeon_spawn_mob(lua, cx, args),
        // `dungeon_clear_regen` (`G/questlua_dungeon.cpp:235-243`). Pushes nothing.
        b"d.clear_regen" => {
            cx.game.dungeon_clear_regen();
            Ok(Some(MultiValue::new()))
        }
        // `dungeon_kill_all` (`G/questlua_dungeon.cpp:1491-1499`). Pushes nothing.
        b"d.kill_all" => {
            cx.game.dungeon_kill_all();
            Ok(Some(MultiValue::new()))
        }
        // `dungeon_set_damage_from_race` (`G/questlua_dungeon.cpp:902-919`): two
        // numbers, the flag then the VID, each as `(DWORD)`. Pushes nothing.
        b"d.set_damage_from_race" => {
            if let Some((flag, vid)) = dungeon_dword_pair(lua, args)? {
                cx.game.dungeon_set_damage_from_race(flag, vid);
            }
            Ok(Some(MultiValue::new()))
        }
        // `dungeon_set_damage_from_affect` (`G/questlua_dungeon.cpp:921-938`): two
        // numbers, the flag then the VID, each as `(DWORD)`. Pushes nothing.
        b"d.set_damage_from_affect" => {
            if let Some((flag, vid)) = dungeon_dword_pair(lua, args)? {
                cx.game.dungeon_set_damage_from_affect(flag, vid);
            }
            Ok(Some(MultiValue::new()))
        }
        _ => Ok(ds_calls(cx, name)),
    }
}

/// The `ds.*` bridged calls: the dragon soul qualification and its windows.
/// Returns `None` for a name this table does not hold.
fn ds_calls(cx: &mut Cx<'_>, name: &[u8]) -> Option<MultiValue> {
    let result: Option<Value> = match name {
        // `ds_is_qualified` (`G/questlua_dragonsoul.cpp:57-70`): a Lua number, 1 or 0.
        b"ds.is_qualified" => Some(Value::Number(f64::from(u8::from(
            cx.game.is_ds_qualified(),
        )))),
        // `ds_give_qualification` (`G/questlua_dragonsoul.cpp:43-55`). Pushes nothing.
        b"ds.give_qualification" => {
            cx.game.give_ds_qualification();
            None
        }
        // `ds_open_refine_window` (`G/questlua_dragonsoul.cpp:14-28`). Pushes nothing.
        b"ds.open_refine_window" => {
            cx.game.open_ds_refine();
            None
        }
        // `ds_open_attr_change_window` (`G/questlua_dragonsoul.cpp:30-41`).
        // Pushes nothing.
        b"ds.open_attr_change_window" => {
            cx.game.open_ds_attr_change();
            None
        }
        _ => return None,
    };
    Some(result.into_iter().collect())
}

/// The bridged server-timer calls (`G/questlua_global.cpp:147-229`): the arms, the
/// clears, and the firing argument. Returns `None` for a name this table does not
/// hold.
fn timer_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let result: Option<Value> = match name {
        // `_set_server_timer` (`G/questlua_global.cpp:147-171`): a string name, a
        // number of seconds, and a number argument or none (0). Pushes nothing.
        b"server_timer" => {
            let name = args.front().cloned().unwrap_or(Value::Nil);
            let secs = args.get(1).cloned().unwrap_or(Value::Nil);
            let arg = args.get(2).cloned().unwrap_or(Value::Nil);
            if let (Some(name), Some(secs)) = (host::c_string(lua, name)?, lua.coerce_number(secs)?)
            {
                let arg = match lua.coerce_number(arg)? {
                    Some(arg) => dword(arg),
                    None => 0,
                };
                cx.game.set_server_timer(name, secs, arg);
            }
            None
        }
        // `_clear_server_timer` (`G/questlua_global.cpp:197-206`): the name as
        // `lua_tostring` reads it (empty for no string) and `(DWORD)` of the
        // argument. Pushes nothing.
        b"clear_server_timer" => {
            let name = host::c_string(lua, args.front().cloned().unwrap_or(Value::Nil))?
                .unwrap_or_default();
            let arg = match lua.coerce_number(args.get(1).cloned().unwrap_or(Value::Nil))? {
                Some(arg) => dword(arg),
                None => 0,
            };
            cx.game.clear_server_timer(&name, arg);
            None
        }
        // `_get_server_timer_arg` (`G/questlua_global.cpp:225-229`).
        b"get_server_timer_arg" => Some(Value::Number(f64::from(cx.timer_arg))),
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// The `game.*` bridged calls that write the world's flags. Returns `None` for a name
/// this table does not hold.
fn game_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let last = args.back().cloned().unwrap_or(Value::Nil);
    let result: Option<Value> = match name {
        // `game_set_event_flag` (`G/questlua_game.cpp:29-37`): a string name and a
        // number value. Pushes nothing.
        b"game.set_event_flag" => {
            if let (Some(name), Some(_)) = (
                host::c_string(lua, first.clone())?,
                lua.coerce_number(last.clone())?,
            ) {
                cx.game.set_event_flag(name, host::c_int(lua, last)?);
            }
            None
        }
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// The `q.*` bridged calls that name the running quest or its blocked PC: the index
/// and name readers, and the other-PC stack. Returns `None` for a name this table
/// does not hold.
fn quest_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let last = args.back().cloned().unwrap_or(Value::Nil);
    let result: Option<Value> = match name {
        b"q.getcurrentquestindex" => {
            // `CQuestManager::GetQuestIndexByName` (`G/questmanager.cpp:1282-1290`): the
            // number of the quest that runs, and 0 for a name no quest has.
            let index = cx
                .quest_indices
                .get(cx.quest.as_slice())
                .copied()
                .unwrap_or(0);
            Some(Value::Number(f64::from(index)))
        }
        b"q.getcurrentquestname" => Some(Value::String(lua.create_string(cx.quest.clone())?)),
        // `quest_begin_other_pc_block` (`G/questlua_quest.cpp:204-210`): `lua_tonumber`
        // of the last argument names the blocked PC, and the stack grows even when
        // the game knows no such online character. Pushes nothing.
        b"q.begin_other_pc_block" => {
            let pid = dword(lua.coerce_number(last)?.unwrap_or(0.0));
            cx.stack.push(cx.current);
            cx.game.begin_other_pc(pid);
            if cx.game.switched_pc() {
                cx.current = pid;
            }
            None
        }
        // `quest_end_other_pc_block` (`G/questlua_quest.cpp:212-217`): the script runs
        // for the PID the matching begin pushed again. Quiet with no open block.
        // Pushes nothing.
        b"q.end_other_pc_block" => {
            if let Some(pid) = cx.stack.pop() {
                cx.current = pid;
            }
            cx.game.end_other_pc();
            None
        }
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// `find_npc_by_vnum` (`G/questlua_global.cpp:792-823`) and the `target.*` names
/// (`G/questlua_target.cpp:45-134`), which mark a character on the client's map for the quest
/// that runs (`GetCurrentQuestIndex`, the number of `m_stCurQuest`, 0 for the chat menu's).
/// `target.pos` is not ported. Returns `None` for a name this table does not hold.
fn target_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let arg = |at: usize| args.get(at).cloned().unwrap_or(Value::Nil);
    let quest = cx
        .quest_indices
        .get(cx.quest.as_slice())
        .copied()
        .unwrap_or(0);
    let result = match name {
        b"find_npc_by_vnum" => {
            // A number names a race, and the answer is the VID of a character of that race on
            // the character's map, or 0; anything else logs and answers 0.
            let vid = if let Some(race) = lua.coerce_number(arg(0))? {
                cx.game.find_npc(dword(race))
            } else {
                tracing::warn!("QUEST invalid argument");
                0
            };
            Some(Value::Number(f64::from(vid)))
        }
        b"target.vid" | b"target.npc" | b"target.pc" => {
            // `target_vid`: text or a number names the target and a number is the VID it
            // points at; else the call logs and does nothing. The VID is read as a `DWORD`,
            // where legacy's `(int)` cast would clip the Rewrite's NPC VIDs, which start at
            // 2^31 (a Divergence). The third argument, text or a number, is the name the
            // client shows, and the fourth, a number cast to `int`, whether it hears of the
            // target at all; without one it does.
            let target = host::c_string(lua, arg(0))?;
            let vid = lua.coerce_number(arg(1))?;
            let (Some(target), Some(vid)) = (target, vid) else {
                tracing::warn!("QUEST invalid argument, quest_index {quest}");
                return Ok(Some(MultiValue::new()));
            };
            let send = match lua.coerce_number(arg(3))? {
                Some(flag) => host::c_int(lua, Value::Number(flag))? != 0,
                None => true,
            };
            cx.game.create_target(&Target {
                quest,
                name: target,
                vid: dword(vid),
                desc: host::c_string(lua, arg(2))?,
                send,
            });
            None
        }
        b"target.delete" => {
            // `target_delete`: text or a number names the target to end; else the call logs.
            if let Some(target) = host::c_string(lua, arg(0))? {
                cx.game.delete_target(quest, Some(&target));
            } else {
                tracing::warn!("QUEST invalid argument, quest_index {quest}");
            }
            None
        }
        b"target.clear" => {
            // `target_clear`: every target of the quest ends.
            cx.game.delete_target(quest, None);
            None
        }
        b"target.id" => {
            // `target_id`: the VID the named target points at, and 0 when there is none; a
            // name that is neither text nor a number logs and answers 0.
            let vid = if let Some(target) = host::c_string(lua, arg(0))? {
                cx.game.target_vid(quest, &target)
            } else {
                tracing::warn!("QUEST invalid argument, quest_index {quest}");
                0
            };
            Some(Value::Number(f64::from(vid)))
        }
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// The names that change what the client's quest window shows of the running quest
/// (`G/questlua_quest.cpp:17-45`). None answers anything; `false` means the name is not one of
/// them.
fn window_api(lua: &Lua, cx: &mut Cx<'_>, name: &[u8], last: Value) -> mlua::Result<bool> {
    match name {
        b"q.set_title" => {
            // `quest_set_title` (`G/questlua_quest.cpp:36-45`): the last argument, text or a
            // number, is the title (`SetCurrentQuestTitle`, `G/questpc.cpp:458-462`), and
            // anything else changes nothing.
            if let Some(title) = host::c_string(lua, last)? {
                cx.window.title = title;
                cx.window.send.set(SendFlags::TITLE);
            }
        }
        b"q.start" => {
            // `SetCurrentQuestStartFlag` (`G/questpc.cpp:215-221`) begins the quest only
            // when `iIndex` is 0 (`SetCurrentQuestBeginFlag`, `:441-450`). `PC::SetQuest`
            // gives every quest that has a number its number, so only the chat menu's
            // quest, which no script names, would begin.
            if cx.window.index == 0 {
                cx.window.start = true;
                cx.window.index = cx
                    .quest_indices
                    .get(cx.quest.as_slice())
                    .copied()
                    .unwrap_or(0);
                cx.window.send.set(SendFlags::BEGIN);
            }
        }
        b"q.done" => {
            // `SetCurrentQuestDoneFlag` (`G/questpc.cpp:223-229`) with
            // `ClearCurrentQuestBeginFlag` (`:433-439`): a quest with a number ends.
            if cx.window.index != 0 {
                cx.window.start = false;
                cx.window.send.set(SendFlags::BEGIN);
            }
        }
        // `quest_set_clock_name` (`G/questlua_quest.cpp:57-66`): the last argument, when
        // string-like, is the clock name; anything else changes nothing.
        b"q.set_clock_name" => {
            if let Some(name) = host::c_string(lua, last)? {
                cx.window.clock_name = name;
                cx.window.send.set(SendFlags::CLOCK_NAME);
            }
        }
        // `quest_set_clock_value` (`:68-77`): the last argument, when number-like, is
        // the clock value (`(int)rint`); anything else changes nothing.
        b"q.set_clock_value" => {
            if let Some(value) = lua.coerce_number(last.clone())? {
                cx.window.clock_value = host::c_int(lua, Value::Number(value.round_ties_even()))?;
                cx.window.send.set(SendFlags::CLOCK_VALUE);
            }
        }
        // `quest_set_counter_name` (`:79-88`): the last argument, when string-like, is
        // the counter name; anything else changes nothing.
        b"q.set_counter_name" => {
            if let Some(name) = host::c_string(lua, last)? {
                cx.window.counter_name = name;
                cx.window.send.set(SendFlags::COUNTER_NAME);
            }
        }
        // `quest_set_counter_value` (`:90-99`): the last argument, when number-like, is
        // the counter value (`(int)rint`); anything else changes nothing.
        b"q.set_counter_value" => {
            if let Some(value) = lua.coerce_number(last.clone())? {
                cx.window.counter_value = host::c_int(lua, Value::Number(value.round_ties_even()))?;
                cx.window.send.set(SendFlags::COUNTER_VALUE);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// A family of API names that answers the names it backs and `None` for any other.
type Family = fn(&Lua, &mut Cx<'_>, &[u8], &MultiValue) -> mlua::Result<Option<MultiValue>>;

/// The window's look: its skin and its left and top images, each answering nothing.
fn look_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let last = args.back().cloned().unwrap_or(Value::Nil);
    match name {
        b"set_skin" | b"setskin" => {
            if let Some(skin) = lua.coerce_number(last)? {
                // `(int)rint(...)`, then the range check; a rounded value is a whole number, so
                // it is a skin exactly when it is closer than a half to one.
                let skin = skin.round_ties_even();
                cx.output.skin = (0..QUEST_SKIN_COUNT)
                    .find(|candidate| (f64::from(*candidate) - skin).abs() < 0.5)
                    .unwrap_or(QUEST_SKIN_NORMAL);
            } else {
                tracing::warn!("QUEST wrong skin index");
            }
        }
        b"setleftimage" | b"settopimage" => {
            if let Some(source) = host::c_string(lua, last)? {
                let tag: &[u8] = if name == b"setleftimage" {
                    b"[LEFTIMAGE src;"
                } else {
                    b"[TOPIMAGE src;"
                };
                cx.output.script.extend_from_slice(tag);
                cx.output.script.extend(source);
                cx.output.script.push(b']');
            }
        }
        _ => return Ok(None),
    }
    Ok(Some(MultiValue::new()))
}

/// The quest item's readers and its socket writer: the vnum, socket, value, count and
/// type the character's held item answers, each 0 with no quest item, and the socket
/// write. Returns `None` for a name this family does not hold.
fn item_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let number = |value: u32| Some(Value::Number(f64::from(value)));
    let result = match name {
        // `item_get_id` (`G/questlua_item.cpp:78-89`): the quest item's ID, or 0 with none.
        b"item.get_id" => number(cx.item),
        // `item_get_cell` (`:21-32`): the quest item's cell, or 0 with none or one the
        // character no longer holds (the `Pc::item` Defect, as `item.get_vnum` reads it).
        b"item.get_cell" => {
            let cell = (cx.item != 0).then(|| cx.game.held_cell(cx.item)).flatten();
            number(u32::from(cell.unwrap_or(0)))
        }
        // `item_select` (`:57-77`) and `item_select_cell` (`:34-56`): the quest item
        // pointer moves to the item of the first argument's ID, or to the one in its
        // base inventory cell (`(DWORD)` of a number, so -1 is 4294967295 and finds
        // nothing). A non-number argument, or one that names nothing, moves nothing;
        // either way the call answers whether it moved.
        b"item.select" => {
            let moved = match lua.coerce_number(first.clone())? {
                Some(id) if dword(id) != 0 && cx.game.held_vnum(dword(id)).is_some() => {
                    cx.item = dword(id);
                    true
                }
                _ => false,
            };
            Some(Value::Boolean(moved))
        }
        b"item.select_cell" => {
            let moved = match lua.coerce_number(first.clone())? {
                Some(cell) => match cx.game.inventory_at(dword(cell)) {
                    Some(id) if id != 0 => {
                        cx.item = id;
                        true
                    }
                    _ => false,
                },
                None => false,
            };
            Some(Value::Boolean(moved))
        }
        // `item_get_vnum` (`G/questlua_item.cpp:138-149`): the quest item's vnum, or 0 when
        // there is none. One the character no longer holds answers 0 (the `Pc::item` Defect).
        b"item.get_vnum" => {
            let held = (cx.item != 0).then(|| cx.game.held_vnum(cx.item)).flatten();
            number(held.unwrap_or(0))
        }
        // `item_get_socket` (`G/questlua_item.cpp:107-123`): the quest item's socket at
        // `(int)` of the first argument, 0 with no quest item, a non-number argument
        // or an index outside 0-5. One the character no longer holds answers 0 (the
        // `Pc::item` Defect, as `item.get_vnum` reads it).
        b"item.get_socket" => {
            let socket = match lua.coerce_number(first.clone())? {
                Some(index) if cx.item != 0 => {
                    let index = host::c_int(lua, Value::Number(index))?;
                    usize::try_from(index)
                        .ok()
                        .filter(|index| *index < 6)
                        .and_then(|index| cx.game.held_socket(cx.item, index))
                }
                _ => None,
            };
            Some(Value::Number(f64::from(socket.unwrap_or(0))))
        }
        // `item_get_value` (`:175-202`): the quest item's proto value at `(int)` of the
        // first argument, 0 with no quest item, no proto, a non-number argument or an
        // index outside 0-5.
        b"item.get_value" => {
            let value = match lua.coerce_number(first.clone())? {
                Some(index) if cx.item != 0 => {
                    let index = host::c_int(lua, Value::Number(index))?;
                    usize::try_from(index)
                        .ok()
                        .filter(|index| *index < 6)
                        .and_then(|index| cx.game.held_value(cx.item, index))
                }
                _ => None,
            };
            Some(Value::Number(f64::from(value.unwrap_or(0))))
        }
        // `item_get_count` (`:259-270`): the quest item's count, 0 with none.
        b"item.get_count" => {
            let count = (cx.item != 0)
                .then(|| cx.game.held_shape(cx.item))
                .flatten();
            number(u32::from(count.map_or(0, |(count, _, _)| count)))
        }
        // `item_get_type` (`:272-284`) and `item_get_sub_type` (`:286-298`): the quest
        // item's proto type pair, 0 with none or no proto.
        b"item.get_type" => {
            let shape = (cx.item != 0)
                .then(|| cx.game.held_shape(cx.item))
                .flatten();
            number(u32::from(shape.map_or(0, |(_, kind, _)| kind)))
        }
        b"item.get_sub_type" => {
            let shape = (cx.item != 0)
                .then(|| cx.game.held_shape(cx.item))
                .flatten();
            number(u32::from(shape.map_or(0, |(_, _, sub)| sub)))
        }
        b"item.set_socket" => {
            set_socket(lua, cx, args)?;
            None
        }
        _ => return item_remove_api(lua, cx, name),
    };
    Ok(Some(result.into_iter().collect()))
}

/// The quest item's removal and its name, size, refined vnum and refine level:
/// `item_remove` (`G/questlua_item.cpp:91-105`), `item_get_name` (`:233-245`),
/// `item_get_size` (`:246-258`), `item_get_refine_vnum` (`:298-310`) and
/// `item_get_level` (`:330-342`). Split out so `item_api` stays under the line lint.
fn item_remove_api(lua: &Lua, cx: &mut Cx<'_>, name: &[u8]) -> mlua::Result<Option<MultiValue>> {
    let number = |value: u32| Some(Value::Number(f64::from(value)));
    let result = match name {
        // `item_remove`: the quest item leaves when the character holds it (the owner
        // check: the held readers answer `None` for one it holds nowhere), and the
        // pointer clears either way (`ClearCurrentItem`). Pushes nothing.
        b"item.remove" => {
            if cx.item != 0 {
                cx.game.remove_quest_item(cx.item);
                cx.item = 0;
            }
            None
        }
        // `item_get_name`: the quest item's locale name, empty with none or no proto.
        b"item.get_name" => {
            let name = (cx.item != 0)
                .then(|| cx.game.held_refine(cx.item))
                .flatten()
                .map_or(Vec::new(), |(name, _, _)| name);
            Some(Value::String(lua.create_string(name)?))
        }
        // `item_get_size`: the quest item's proto size, 0 with none or no proto.
        b"item.get_size" => {
            let size = (cx.item != 0)
                .then(|| cx.game.held_refine(cx.item))
                .flatten();
            number(u32::from(size.map_or(0, |(_, size, _)| size)))
        }
        // `item_get_refine_vnum`: the quest item's proto refined vnum, 0 with none
        // or no proto.
        b"item.get_refine_vnum" => {
            let refined = (cx.item != 0)
                .then(|| cx.game.held_refine(cx.item))
                .flatten();
            number(refined.map_or(0, |(_, _, refined)| refined))
        }
        // `item_get_level`: the quest item's refine level, 0 with none, no proto or
        // no `+`.
        b"item.get_level" => {
            let level = (cx.item != 0)
                .then(|| cx.game.held_refine_level(cx.item))
                .flatten();
            Some(Value::Number(f64::from(level.unwrap_or(0))))
        }
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
}

/// `number` (`G/questlua_global.cpp`): a random roll over the first two arguments,
/// both number-like, and 0 when either is not.
fn drawn_number(lua: &Lua, cx: &mut Cx<'_>, args: &MultiValue, first: Value) -> mlua::Result<i32> {
    let second = args.get(1).cloned().unwrap_or(Value::Nil);
    let both =
        lua.coerce_number(first.clone())?.is_some() && lua.coerce_number(second.clone())?.is_some();
    if !both {
        return Ok(0);
    }
    let from = host::c_int(lua, first)?;
    let to = host::c_int(lua, second)?;
    Ok(cx.game.number(from, to))
}

/// `item_name`/`mob_name`: the name the game holds for the first argument's vnum,
/// and the empty string for a non-number or an unknown vnum.
fn named_thing(lua: &Lua, cx: &mut Cx<'_>, name: &[u8], first: Value) -> mlua::Result<Vec<u8>> {
    let text = match lua.coerce_number(first)? {
        Some(vnum) if name == b"item_name" => cx.game.item_name(dword(vnum)),
        Some(vnum) => cx.game.mob_name(dword(vnum)),
        None => None,
    };
    Ok(text.unwrap_or_default())
}

fn api(lua: &Lua, cx: &mut Cx<'_>, name: &[u8], args: &MultiValue) -> mlua::Result<MultiValue> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let last = args.back().cloned().unwrap_or(Value::Nil);
    let number = |value: u32| Some(Value::Number(f64::from(value)));
    if let TextOutcome::Hit(result) = text_api(lua, cx, name, args, last.clone())? {
        return Ok(result.into_iter().collect());
    }
    let families: [Family; 17] = [
        look_api,
        pc_api,
        item_api,
        target_api,
        guild_api,
        party_api,
        dungeon_api,
        quest_api,
        horse_api,
        mount_api,
        attr67_api,
        rank_api,
        timer_api,
        game_api,
        building_api,
        meley_api,
        ship_api,
    ];
    for family in families {
        if let Some(answered) = family(lua, cx, name, args)? {
            return Ok(answered);
        }
    }
    if window_api(lua, cx, name, last.clone())? {
        return Ok(MultiValue::new());
    }
    let result = match name {
        b"getnpcid" => number(npc_id_of(lua, cx, last)?),
        b"number" => Some(Value::Number(f64::from(drawn_number(
            lua, cx, args, first,
        )?))),
        // `_refine_pick` and `_fish_real_refine_rod` (`G/questlua_global.cpp:450-478`): the
        // tool in inventory cell `(BYTE)` of the last argument (`lua_tonumber`, so a
        // non-number is cell 0) runs `RealRefinePick` or `RealRefineRod`; 0, 1 or 2 is pushed.
        b"__refine_pick" | b"__fish_real_refine_rod" => {
            let cell = dword(lua.coerce_number(last)?.unwrap_or(0.0)).to_le_bytes()[0];
            let rod = name == b"__fish_real_refine_rod";
            number(u32::from(cx.game.refine_tool(cell, rod)))
        }
        b"item_name" | b"mob_name" => Some(Value::String(
            lua.create_string(named_thing(lua, cx, name, first)?)?,
        )),
        b"get_time" | b"get_global_time" => number(cx.game.global_time()),
        // The time, test-server and find-PC arms (see `time_find_api`).
        b"time_to_str" | b"is_test_server" | b"find_pc_by_name" => {
            return time_find_api(lua, cx, name, first, last);
        }
        // The game, NPC and warp arms (see `game_npc_api`).
        b"game.get_event_flag"
        | b"game.request_make_guild"
        | b"game.drop_item"
        | b"game.drop_item_with_ownership"
        | b"game.open_gaya"
        | b"game.open_gaya_market"
        | b"game.open_aura_absorb_window"
        | b"game.open_aura_growth_window"
        | b"game.open_aura_evolve_window"
        | b"warp_to_village"
        | b"npc.getrace"
        | b"npc.get_race"
        | b"npc.get_empire"
        | b"npc.get_guild" => {
            return game_npc_api(lua, cx, name, args, first);
        }
        b"set_state" | b"q.setstate" | b"q.set_state" => {
            match host::c_string(lua, last)? {
                Some(state) if !state.is_empty() => {
                    cx.pending_state = Some(state);
                }
                _ => tracing::warn!("QUEST state name is empty"),
            }
            None
        }
        // The quiet globals (see `quiet_api`).
        b"LC"
        | b"notice_all"
        | b"set_quest_state"
        | b"command"
        | b"add_ox_quiz"
        | b"start_event"
        | b"end_event"
        | b"temple_ochao.initialize" => {
            return quiet_api(lua, cx, name, args, first);
        }
        // The quest-log pair (see `quest_log`).
        b"char_log" | b"sys_err" => {
            quest_log(lua, cx, name, args, &first)?;
            None
        }
        _ => {
            return Err(mlua::Error::runtime(format!(
                "not ported: {}",
                String::from_utf8_lossy(name)
            )))
        }
    };
    Ok(result.into_iter().collect())
}

/// `getnpcid` (`G/questlua_global.cpp:289-296`): the ID of the NPC the last argument
/// names, 0 for a non-string or an unknown name.
fn npc_id_of(lua: &Lua, cx: &Cx<'_>, last: Value) -> mlua::Result<u32> {
    Ok(host::c_string(lua, last)?
        .and_then(|name| cx.npc_ids.get(&name).copied())
        .unwrap_or(0))
}

/// `get_locale` (`G/locale.cpp:30-56`): the two letters of the language byte.
/// `LOCALE_YMIR` (0) and anything past `LOCALE_TR` (11) answer `en`, as the
/// `default` arm does.
fn locale_code(language: u8) -> &'static [u8] {
    match language {
        2 => b"pt",
        3 => b"es",
        4 => b"fr",
        5 => b"de",
        6 => b"ro",
        7 => b"pl",
        8 => b"it",
        9 => b"cz",
        10 => b"hu",
        11 => b"tr",
        _ => b"en",
    }
}

/// The quiet globals: `LC` (`G/questlua_global.cpp:44-54`), `_notice_all`
/// (`:944-991`), `_set_quest_state` (`:828-862`), `_do_command` (`:730-741`),
/// `_add_ox_quiz` (`:1144-1156`), `_start_event` (`:1636-1665`), `_end_event`
/// (`:1668-1686`) and `temple_ochao_initialize`
/// (`G/questlua_TempleOchao.cpp:15-20`). Split out so `api` stays under the line lint.
fn quiet_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
    first: Value,
) -> mlua::Result<MultiValue> {
    let result = match name {
        // `_get_locale`: the descriptor's language as two letters.
        b"LC" => Some(Value::String(
            lua.create_string(locale_code(cx.game.language()))?,
        )),
        // `_notice_all`: at most six parts (`MAX_QUEST_NOTICE_ARGS + 1`); more only
        // logs. Each part is `snprintf %s`, so a missing one is `(null)` and a number
        // formats. Pushes nothing.
        b"notice_all" => {
            if args.len() > 6 {
                tracing::warn!("QUEST invalid argument.");
            } else {
                let mut parts = Vec::new();
                for index in 0..6 {
                    let part = args.get(index).cloned().unwrap_or(Value::Nil);
                    let text = host::c_string(lua, part)?.unwrap_or_else(|| b"(null)".to_vec());
                    parts.push(text);
                }
                cx.game.notice_all(parts);
            }
            None
        }
        // `_set_quest_state`: two strings name the quest and its state, else quiet.
        // The running quest moves at once through `pending_state`, as `pqs->st` does;
        // another quest's move is reserved for the end, as `m_QuestStateChange` is.
        // Pushes nothing.
        b"set_quest_state" => {
            let quest = host::c_string(lua, first.clone())?;
            let state = host::c_string(lua, args.get(1).cloned().unwrap_or(Value::Nil))?;
            if let (Some(quest), Some(state)) = (quest, state) {
                if quest == cx.quest {
                    cx.pending_state = Some(state);
                } else {
                    let index = cx.quest_indices.get(&quest).copied().unwrap_or(0);
                    cx.pending_others.push((index, state));
                }
            }
            None
        }
        // `_do_command`: a string runs as the character; anything else is quiet.
        // Pushes nothing.
        b"command" => {
            if let Some(line) = host::c_string(lua, first)? {
                cx.game.quest_command(line);
            }
            None
        }
        // `_add_ox_quiz` (`G/questlua_global.cpp:1144-1156`): `(int)` of the first
        // argument is the level (`unsigned char`, so its low byte), a string-like
        // second is the question (a number formats), and the third's truth is the
        // answer. The (n+1)th push of `return 1` is the level itself. Quiet with no
        // question, as `AddQuiz` would fault on the NULL.
        b"add_ox_quiz" => {
            let level = u32::from_le_bytes(host::c_int(lua, first)?.to_le_bytes()).to_le_bytes()[0];
            let question = host::c_string(lua, args.get(1).cloned().unwrap_or(Value::Nil))?;
            if let Some(question) = question {
                let answer = !matches!(
                    args.get(2).cloned().unwrap_or(Value::Nil),
                    Value::Nil | Value::Boolean(false)
                );
                cx.game.add_ox_quiz(level, question, answer);
            }
            Some(Value::Number(f64::from(level)))
        }
        // `_start_event` (`G/questlua_global.cpp:1636-1665`): a number-like first
        // argument names the event, number-like second and third its two values
        // (`lua_isnumber`, so a numeric string counts and anything else reads 0).
        // Pushes nothing; a non-number first pushes 0 and starts nothing.
        b"start_event" => {
            let Some(index) = lua.coerce_number(first)? else {
                return Ok(MultiValue::from_vec(vec![Value::Number(0.0)]));
            };
            let value0 = lua
                .coerce_number(args.get(1).cloned().unwrap_or(Value::Nil))?
                .unwrap_or(0.0);
            let value1 = lua
                .coerce_number(args.get(2).cloned().unwrap_or(Value::Nil))?
                .unwrap_or(0.0);
            cx.game.start_event(
                host::c_int(lua, Value::Number(index))?,
                host::c_int(lua, Value::Number(value0))?,
                host::c_int(lua, Value::Number(value1))?,
            );
            None
        }
        // `_end_event` (`G/questlua_global.cpp:1668-1686`): a number-like first
        // argument names the event. Pushes nothing; a non-number first pushes 0.
        b"end_event" => {
            let Some(index) = lua.coerce_number(first)? else {
                return Ok(MultiValue::from_vec(vec![Value::Number(0.0)]));
            };
            cx.game.end_event(host::c_int(lua, Value::Number(index))?);
            None
        }
        // `temple_ochao_initialize`: the temple prepares. Pushes nothing.
        b"temple_ochao.initialize" => {
            cx.game.prepare_temple_ochao();
            None
        }
        _ => return Ok(MultiValue::new()),
    };
    Ok(result.into_iter().collect())
}

/// The time, test-server and find-PC arms: `_time_to_str`
/// (`G/questlua_global.cpp:65-70`), `_is_test_server` (`:298-302`) and `_find_pc`
/// (`:743-757`). Split out so `api` stays under the line lint.
fn time_find_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    first: Value,
    last: Value,
) -> mlua::Result<MultiValue> {
    let result = match name {
        // `_time_to_str`: `asctime(gmtime())` of `(time_t)` of the last argument, a
        // number or a text one.
        b"time_to_str" => {
            let stamp = lua.coerce_number(last)?;
            Some(Value::String(lua.create_string(time_to_str(stamp))?))
        }
        // `_is_test_server`.
        b"is_test_server" => Some(Value::Boolean(cx.game.test_server())),
        // `_find_pc`: the VID of the online PC the first argument names, and 0 for
        // a non-string or an unknown name.
        b"find_pc_by_name" => {
            let vid = match first {
                Value::String(text) => host::c_string(lua, Value::String(text.clone()))?
                    .map_or(0, |name| cx.game.find_pc_vid(&name)),
                _ => 0,
            };
            Some(Value::Number(f64::from(vid)))
        }
        _ => None,
    };
    Ok(result.into_iter().collect())
}

/// The `building.*` calls (`G/questlua_building.cpp`): the land id read, the info
/// triple, the owner write, the owned read, and the object rebuild. Split out so `api`
/// stays under the line lint.
///
///
///
///
///
/// The `MeleyLair.*` lobby calls (`G/questlua_MeleyLair.cpp`): the two constant
/// pairs, the register and enter pairs, the map read, and the two queued warps.
/// Split out so `api` stays under the line lint.
/// It cannot fail, but the [`Family`] table calls through `Result`, so it keeps the wrap.
#[allow(clippy::unnecessary_wraps)]
fn meley_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let _ = args;
    let pair = |first: f64, second: f64| {
        MultiValue::from_vec(vec![Value::Number(first), Value::Number(second)])
    };
    // `MeleyLair_GetRequirments` (`:18-23`): the minimum guild level and the ladder
    // cost (`MeleyLair.h:18-20`).
    if name == b"MeleyLair.GetRequirments" {
        return Ok(Some(pair(3.0, 600.0)));
    }
    // `MeleyLair_GetPartecipantsLimit` (`:25-29`): 16 (`MeleyLair.h:19`).
    if name == b"MeleyLair.GetPartecipantsLimit" {
        return Ok(Some(MultiValue::from_vec(vec![Value::Number(16.0)])));
    }
    // `MeleyLair_GetSubMapIndex` (`:31-35`): 62 (`MeleyLair.h:17`).
    if name == b"MeleyLair.GetSubMapIndex" {
        return Ok(Some(MultiValue::from_vec(vec![Value::Number(62.0)])));
    }
    // `MeleyLair_Register` (`:37-49`): the judged (`iRes1`, `iRes2`) pair. The
    // register runs mid-script, beside its script's world, as `guild_call` does.
    if name == b"MeleyLair.Register" {
        let (first, second) = cx.game.meley_register();
        return Ok(Some(pair(f64::from(first), f64::from(second))));
    }
    // `MeleyLair_IsRegistered` (`:51-68`): the boolean and the channel (0 with no
    // character, as the `NULL` arm pushes).
    if name == b"MeleyLair.IsRegistered" {
        let (registered, channel) = cx.game.meley_is_registered();
        return Ok(Some(MultiValue::from_vec(vec![
            Value::Boolean(registered),
            Value::Number(f64::from(channel)),
        ])));
    }
    // `MeleyLair_Enter` (`:70-87`): the boolean and the limit (1 with no
    // character, as the `NULL` arm pushes).
    if name == b"MeleyLair.Enter" {
        let (entered, limit) = cx.game.meley_enter();
        return Ok(Some(MultiValue::from_vec(vec![
            Value::Boolean(entered),
            Value::Number(f64::from(limit)),
        ])));
    }
    // `MeleyLair_IsMeleyMap` (`:89-97`): false with no character, as the `NULL`
    // arm pushes.
    if name == b"MeleyLair.IsMeleyMap" {
        return Ok(Some(MultiValue::from_vec(vec![Value::Boolean(
            cx.game.is_meley_map(),
        )])));
    }
    // `MeleyLair_Check` (`:99-107`) and `MeleyLair_Leave` (`:109-117`): queued,
    // pushing nothing.
    if name == b"MeleyLair.Check" {
        cx.game.meley_check();
        return Ok(Some(MultiValue::new()));
    }
    if name == b"MeleyLair.Leave" {
        cx.game.meley_leave();
        return Ok(Some(MultiValue::new()));
    }
    // `MeleyLair_CanGetReward` (`:119-131`): the shadow read.
    if name == b"MeleyLair.CanGetReward" {
        return Ok(Some(MultiValue::from_vec(vec![Value::Boolean(
            cx.game.meley_can_reward(),
        )])));
    }
    // `MeleyLair_Reward` (`:133-145`): `lua_isnumber` of the one argument, queued,
    // pushing nothing; a non-number queues nothing, as the early return does.
    if name == b"MeleyLair.Reward" {
        let Some(number) = lua.coerce_number(args.front().cloned().unwrap_or(Value::Nil))? else {
            return Ok(Some(MultiValue::new()));
        };
        cx.game.meley_reward(dword(number).to_le_bytes()[0]);
        return Ok(Some(MultiValue::new()));
    }
    Ok(None)
}

/// The `ship_defense_mgr.*` lobby calls (`G/questlua_shipdefense_mgr.cpp`): the four
/// `EInstance` constant booleans, the created/running/join reads, and the six queued
/// writes. Split out so `api` stays under the line lint.
///
/// It cannot fail, but the [`Family`] table calls through `Result`, so it keeps the wrap.
#[allow(clippy::unnecessary_wraps)]
fn ship_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let yes = |flag: bool| MultiValue::from_vec(vec![Value::Boolean(flag)]);
    // `ship_defense_mgr_create` (`:28-42`), `ship_defense_mgr_start` (`:44-62`),
    // `ship_defense_mgr_join` (`:64-74`), `ship_defense_mgr_leave` (`:76-86`) and
    // `ship_defense_mgr_land` (`:88-98`): queued, pushing nothing.
    if name == b"ship_defense_mgr.create" {
        cx.game.ship_create();
        return Ok(Some(MultiValue::new()));
    }
    if name == b"ship_defense_mgr.start" {
        cx.game.ship_start();
        return Ok(Some(MultiValue::new()));
    }
    if name == b"ship_defense_mgr.join" {
        cx.game.ship_join();
        return Ok(Some(MultiValue::new()));
    }
    if name == b"ship_defense_mgr.leave" {
        cx.game.ship_leave();
        return Ok(Some(MultiValue::new()));
    }
    if name == b"ship_defense_mgr.land" {
        cx.game.ship_land();
        return Ok(Some(MultiValue::new()));
    }
    // `ship_defense_mgr_is_created` (`:100-111`) and `ship_defense_mgr_is_running`
    // (`:113-124`): the shadow pair.
    if name == b"ship_defense_mgr.is_created" {
        return Ok(Some(yes(cx.game.ship_is_created())));
    }
    if name == b"ship_defense_mgr.is_running" {
        return Ok(Some(yes(cx.game.ship_is_running())));
    }
    // `ship_defense_mgr_need_party` (`:126-130`), `ship_defense_mgr_need_ticket`
    // (`:132-136`), `ship_defense_mgr_spawn_wood_repair` (`:138-142`) and
    // `ship_defense_mgr_require_cooldown` (`:144-148`): the `EInstance` constants
    // (`ShipDefense.h:19-24`), all true in this build.
    if name == b"ship_defense_mgr.need_party"
        || name == b"ship_defense_mgr.need_ticket"
        || name == b"ship_defense_mgr.spawn_wood_repair"
        || name == b"ship_defense_mgr.require_cooldown"
    {
        return Ok(Some(yes(true)));
    }
    // `ship_defense_mgr_set_alliance_hp_pct` (`:142-159`): `lua_isnumber` of the one
    // argument, queued, pushing nothing; a non-number logs and queues nothing.
    if name == b"ship_defense_mgr.set_alliance_hp_pct" {
        let Some(number) = lua.coerce_number(args.front().cloned().unwrap_or(Value::Nil))? else {
            tracing::warn!("QUEST invalid argument");
            return Ok(Some(MultiValue::new()));
        };
        cx.game.ship_heal(dword(number).to_le_bytes()[0]);
        return Ok(Some(MultiValue::new()));
    }
    // `ship_defense_mgr_can_join` (`:161-174`): the shadow read (false with no
    // character, as the `NULL` arm pushes).
    if name == b"ship_defense_mgr.can_join" {
        return Ok(Some(yes(cx.game.ship_can_join())));
    }
    Ok(None)
}

fn building_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
) -> mlua::Result<Option<MultiValue>> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let second = args.get(1).cloned().unwrap_or(Value::Nil);
    let third = args.get(2).cloned().unwrap_or(Value::Nil);
    // `lua_isnumber` of the three fixed arguments: a non-number pushes 0 (`:15-21`).
    if name == b"building.get_land_id" {
        let id = if let (Some(map), Some(x), Some(y)) = (
            lua.coerce_number(first)?,
            lua.coerce_number(second)?,
            lua.coerce_number(third)?,
        ) {
            cx.game
                .land_id_at(dword_long(map), dword_long(x), dword_long(y))
        } else {
            tracing::warn!("QUEST invalid argument");
            0
        };
        return Ok(Some(MultiValue::from_vec(vec![Value::Number(f64::from(
            id,
        ))])));
    }
    // `building_get_land_info` (`:31-58`): a non-number logs and the triple answers
    // 10^9; a number with no land answers the same.
    if name == b"building.get_land_info" {
        let triple = if let Some(land) = lua.coerce_number(first)? {
            cx.game.land_info(dword(land))
        } else {
            tracing::warn!("QUEST invalid argument");
            None
        };
        let Some((price, owner, limit)) = triple else {
            return Ok(Some(MultiValue::from_vec(vec![
                Value::Number(1_000_000_000.0),
                Value::Number(1_000_000_000.0),
                Value::Number(1_000_000_000.0),
            ])));
        };
        return Ok(Some(MultiValue::from_vec(vec![
            Value::Number(f64::from(price)),
            Value::Number(f64::from(owner)),
            Value::Number(f64::from(limit)),
        ])));
    }
    // `building_set_land_owner` (`:60-80`): two numbers, else a log. Pushes nothing.
    if name == b"building.set_land_owner" {
        if let (Some(land), Some(guild)) = (lua.coerce_number(first)?, lua.coerce_number(second)?) {
            cx.game.set_land_owner(dword(land), dword(guild));
        } else {
            tracing::warn!("QUEST invalid argument");
        }
        return Ok(Some(MultiValue::from_vec(Vec::new())));
    }
    // `building_has_land` (`:83-118`): a non-number logs and pushes true; else
    // whether the guild owns a land.
    if name == b"building.has_land" {
        let has = if let Some(guild) = lua.coerce_number(first)? {
            cx.game.guild_has_land(dword(guild))
        } else {
            tracing::warn!("QUEST invalid argument");
            true
        };
        return Ok(Some(MultiValue::from_vec(vec![Value::Boolean(has)])));
    }
    // `building_reconstruct` (`:126-153`): `lua_tonumber` of the first argument names
    // the new building; a non-number is 0. Pushes nothing.
    if name == b"building.reconstruct" {
        let vnum = dword(lua.coerce_number(first)?.unwrap_or(0.0));
        cx.game.reconstruct_object(cx.npc_vid, vnum);
        return Ok(Some(MultiValue::from_vec(Vec::new())));
    }
    Ok(None)
}

/// The game, NPC and village-warp arms: `game.get_event_flag`
/// (`G/questlua_game.cpp`), `game_request_make_guild` (`:51-62`),
/// `game_drop_item_with_ownership`, the Gaya pair (`:95-126`) and the aura trio
/// (`:312-351`), `_warp_to_village` (`G/questlua_global.cpp:1060-1071`),
/// `npc_getrace`, `npc_get_empire` (`G/questlua_npc.cpp:43-52`) and `npc_get_guild`
/// (`:60-70`). Split out so `api` stays under the line lint.
fn game_npc_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
    first: Value,
) -> mlua::Result<MultiValue> {
    let result = match name {
        b"game.get_event_flag" => {
            let value = host::c_string(lua, first)?
                .and_then(|flag| cx.event_flags.get(&flag).copied())
                .unwrap_or(0);
            Some(Value::Number(f64::from(value)))
        }
        // `game_set_event_flag` (`G/questlua_game.cpp:29-37`): a string name and a
        // number value. Pushes nothing.
        // `game_request_make_guild` (`G/questlua_game.cpp:51-62`): the client hears
        // `HEADER_GC_REQUEST_MAKE_GUILD`. It pushes nothing.
        b"game.request_make_guild" => {
            cx.game.request_make_guild();
            None
        }
        // `game_drop_item` (`G/questlua_game.cpp:138-166`): `(DWORD)` of the first
        // argument is the vnum, `(int)` of the second the count. Pushes nothing.
        b"game.drop_item" => {
            let vnum = u32::from_le_bytes(host::c_int(lua, first)?.to_le_bytes());
            let count = host::c_int(lua, args.get(1).cloned().unwrap_or(Value::Nil))?;
            cx.game.drop_item(vnum, count);
            None
        }
        // `game_drop_item_with_ownership` (see `drop_with_ownership`): pushes nothing.
        b"game.drop_item_with_ownership" => {
            drop_with_ownership(lua, cx, args)?;
            None
        }
        // The Gaya pair and the aura trio: each opens its window. Push nothing.
        b"game.open_gaya" => {
            cx.game.open_gaya();
            None
        }
        b"game.open_gaya_market" => {
            cx.game.open_gaya_market();
            None
        }
        b"game.open_aura_absorb_window" => {
            cx.game.open_aura_absorb(cx.npc_vid);
            None
        }
        b"game.open_aura_growth_window" => {
            cx.game.open_aura_growth(cx.npc_vid);
            None
        }
        b"game.open_aura_evolve_window" => {
            cx.game.open_aura_evolve(cx.npc_vid);
            None
        }
        // `_warp_to_village` (`G/questlua_global.cpp:1060-1071`): pushes nothing.
        b"warp_to_village" => {
            cx.game.warp_to_village();
            None
        }
        b"npc.getrace" | b"npc.get_race" => Some(Value::Number(f64::from(
            cx.game.race_of(cx.npc_vid).unwrap_or(0),
        ))),
        // `npc_get_empire` (`G/questlua_npc.cpp:43-52`): the NPC's empire, 0 with no NPC.
        b"npc.get_empire" => Some(Value::Number(f64::from(u32::from(
            cx.game.empire_of(cx.npc_vid).unwrap_or(0),
        )))),
        // `npc_get_guild` (`G/questlua_npc.cpp:60-70`): the NPC's guild ID, 0 with no NPC
        // or no guild. No NPC holds one: no building stands.
        b"npc.get_guild" => Some(Value::Number(f64::from(cx.game.npc_guild(cx.npc_vid)))),
        _ => None,
    };
    Ok(result.into_iter().collect())
}

/// The quest-log pair: `_char_log` (`G/questlua_global.cpp:316-332`) and `_syserr`
/// (`:385-403`). Split out so `api` stays under the line lint.
fn quest_log(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    args: &MultiValue,
    first: &Value,
) -> mlua::Result<()> {
    match name {
        // `_char_log` (`G/questlua_global.cpp:316-332`): `(DWORD)` of a number-like first
        // argument is `what`, a string or number-like second is `how`, and any
        // string-like third (`lua_tostring`, so a number formats) is `hint`. Pushes
        // nothing.
        b"char_log" => {
            let what = lua.coerce_number(first.clone())?.map_or(0, dword);
            let how = host::c_string(lua, args.get(1).cloned().unwrap_or(Value::Nil))?
                .unwrap_or_default();
            let hint = host::c_string(lua, args.get(2).cloned().unwrap_or(Value::Nil))?
                .unwrap_or_default();
            cx.game.char_log(what, &how, &hint);
        }
        // `_syserr` (`G/questlua_global.cpp:385-403`): a string-like first argument
        // (`lua_isstring`, so a number formats) logs and chats; anything else is
        // quiet. Pushes nothing.
        b"sys_err" => {
            if let Some(text) = host::c_string(lua, first.clone())? {
                cx.game.sys_err(&cx.quest, &text);
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::OnceLock;

    fn locale_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe")
    }

    fn load() -> Manager {
        Manager::load(&locale_dir()).unwrap_or_else(|error| panic!("{error}"))
    }

    const PID: u32 = 7;
    const VID: u32 = 1234;
    const OX_MANAGER: u32 = 20011;

    #[derive(Default)]
    #[allow(clippy::struct_excessive_bools)]
    struct Fake {
        level: u8,
        playtime: i32,
        /// Every `give_item` call, in order.
        gives: Vec<(u32, u16)>,
        /// What `give_item` answers.
        given: u32,
        /// The vnums `item_vnum` finds by name.
        item_vnums: BTreeMap<Vec<u8>, u32>,
        /// What `count_item` answers, by vnum.
        counts: BTreeMap<u32, i32>,
        /// Every `remove_item` call, in order.
        removals: Vec<(u32, u32)>,
        skill_group: u8,
        /// The vnum `held_vnum` finds by item ID.
        held: BTreeMap<u32, u32>,
        /// The cell `held_cell` finds by item ID.
        held_cells: BTreeMap<u32, u16>,
        /// The ID `inventory_at` finds by base cell.
        inventory: BTreeMap<u32, u32>,
        /// Every `ore_refine` call, in order.
        melts: Vec<(u32, i32, i32, Option<u32>)>,
        /// What `ore_refine` answers.
        melted: bool,
        job: u8,
        /// The levels `set_skill_level` set, which `skill_level` then reads.
        levels: BTreeMap<u32, u8>,
        /// Every `set_skill_level` call, in order.
        set_levels: Vec<(u32, u8)>,
        /// The character's stored alignment, which `change_alignment` moves.
        alignment: i32,
        /// Every `change_alignment` amount, in order.
        alignment_changes: Vec<i32>,
        /// Whether the character holds the no-delay book affect.
        skill_book_no_delay: bool,
        /// Every `remove_skill_book_no_delay` call.
        skill_book_removals: usize,
        /// Every `set_socket` call, in order.
        sockets: Vec<(u32, usize, i32)>,
        races: BTreeMap<u32, u32>,
        chats: Vec<(ChatKind, Vec<u8>)>,
        scripts: Vec<(u8, Vec<u8>)>,
        draws: Vec<(i32, i32)>,
        learned: BTreeSet<u32>,
        guild_calls: Vec<(Vec<u8>, Vec<GuildArg>)>,
        /// What `get_reserve_war_table` answers once.
        reserve_table: Vec<Vec<f64>>,
        map_index: i32,
        war_map: i32,
        gold: u64,
        gold_changes: Vec<i64>,
        hides: Vec<(u8, bool)>,
        empire: u8,
        hp: i32,
        max_hp: i32,
        sp: i32,
        max_sp: i32,
        exp: u32,
        next_exp: u32,
        race: u16,
        char_name: Vec<u8>,
        char_vid: u32,
        leadership: u8,
        position: (i32, i32, i32, i32),
        worn: BTreeMap<u8, u32>,
        empires: BTreeMap<u32, u8>,
        npc_guilds: BTreeMap<u32, u32>,
        lands_at: BTreeMap<(i32, i32, i32), u32>,
        land_infos: BTreeMap<u32, (u32, u32, u8)>,
        land_guilds: BTreeSet<u32>,
        set_owners: Vec<(u32, u32)>,
        reconstructs: Vec<(u32, u32)>,
        meley_registers: usize,
        meley_registered: (i32, i32),
        meley_registered_flag: (bool, u8),
        meley_enters: usize,
        meley_entered: (bool, i32),
        meley_map: bool,
        meley_checks: usize,
        meley_leaves: usize,
        meley_rewardable: bool,
        meley_rewards: Vec<u8>,
        ship_creates: usize,
        ship_starts: usize,
        ship_joins: usize,
        ship_leaves: usize,
        ship_lands: usize,
        ship_created: bool,
        ship_running: bool,
        ship_heals: Vec<u8>,
        ship_joinable: bool,
        player_id: u32,
        account_table_id: u32,
        empty_cells: u32,
        trading: bool,
        busy: bool,
        exp2: Vec<u32>,
        skill_groups: Vec<u8>,
        local_warps: Vec<(i32, i32, i32)>,
        killee: (i32, i32),
        drops_plain: Vec<(u32, i32)>,
        gm_level: u8,
        /// Whether `has_empty_inventory` answers true, by vnum.
        room: BTreeMap<u32, bool>,
        /// The sockets `held_socket` finds by item ID.
        held_sockets: BTreeMap<(u32, usize), i32>,
        /// The values `held_value` finds by item ID.
        held_values: BTreeMap<(u32, usize), i32>,
        /// The shape `held_shape` finds by item ID.
        held_shapes: BTreeMap<u32, (u16, u8, u8)>,
        /// The name, size and refined vnum `held_refine` finds by item ID.
        held_refines: BTreeMap<u32, (Vec<u8>, u8, u32)>,
        /// The refine level `held_refine_level` finds by item ID.
        held_levels: BTreeMap<u32, i32>,
        /// Every `remove_quest_item` call, in order.
        removed_quest_items: Vec<u32>,
        guild_id: u32,
        is_guild_master: bool,
        /// Whether `guild_known` names the guild.
        guild_known: bool,
        /// Whether the deployment runs in test-server mode.
        test_server: bool,
        /// The language byte `language` answers.
        language: u8,
        /// Every `notice_all` call's parts, in order.
        notices: Vec<Vec<Vec<u8>>>,
        /// Every `prepare_temple_ochao` call.
        temple_prepares: usize,
        /// Every `quest_command` call's line, in order.
        quest_commands: Vec<Vec<u8>>,
        /// Every `add_ox_quiz` call, in order.
        ox_quizzes: Vec<(u8, Vec<u8>, bool)>,
        /// The `open_gaya` calls.
        gaya_opens: usize,
        /// The `open_gaya_market` calls.
        gaya_market_opens: usize,
        /// The aura window opens, in order: the window (0 absorb, 1 growth, 2
        /// evolve) and the opener's VID.
        aura_opens: Vec<(u8, u32)>,
        /// Every `start_event` call, in order.
        event_starts: Vec<(i32, i32, i32)>,
        /// Every `end_event` call, in order.
        event_ends: Vec<i32>,
        /// The VID `find_pc_vid` answers for each PC name.
        pcs: BTreeMap<Vec<u8>, u32>,
        /// The PID `pid_of` answers for each VID.
        vids: BTreeMap<u32, u32>,
        /// Every `confirm_to` call, in order.
        confirms: Vec<(u32, Vec<u8>, i32, u32)>,
        /// Every `arm_confirm_timeout` call, in order.
        confirm_timeouts: Vec<(u32, i32)>,
        /// Every `destroy_guild` call.
        disbands: usize,
        /// Every `remove_from_guild` call.
        leaves: usize,
        make_guild_asks: usize,
        any_war: f64,
        blocked: Vec<u8>,
        infos: Vec<QuestInfo>,
        /// The VID `find_npc` answers for each race on the character's map.
        npcs: BTreeMap<u32, u32>,
        /// The live targets, in the order they were made.
        targets: Vec<Target>,
        /// Every `create_target` call, in order.
        created: Vec<Target>,
        /// Every `delete_target` call, in order.
        deleted: Vec<(u32, Option<Vec<u8>>)>,
        /// Every `warp` call, in order.
        warps: Vec<(i32, i32, i32)>,
        /// What [`Game::war_observe`] was asked.
        observes: Vec<(u32, u32)>,
        /// Whether `IsHack` refuses the character.
        hack: bool,
        /// Every `drop_with_ownership` call, in order.
        drops: Vec<(u32, i32, Option<i32>)>,
        /// The character's conqueror level.
        conqueror_level: u8,
        /// The character's conqueror experience.
        conqueror_exp: u32,
        /// The `warp_to_village` calls.
        villages: u32,
        /// Whether `CanWarp` passes for the character.
        able: bool,
        /// The `set_conqueror` calls.
        conquerors: u32,
        /// Every `open_sash` call's combination flag, in order.
        sash_opens: Vec<bool>,
        /// The `open_changelook` calls.
        changelooks: u32,
        /// Every `mining` call's NPC, in order.
        minings: Vec<u32>,
        /// Every `refine_tool` call's cell and rod flag, in order.
        tool_refines: Vec<(u8, bool)>,
        /// The horse the readers read.
        horse: HorseView,
        /// Every `set_horse_level` call's level, in order.
        horse_levels: Vec<i32>,
        /// The `feed_horse` calls.
        horse_feeds: u32,
        /// The `summon_horse`, `unsummon_horse` and `revive_horse` calls, in order.
        horse_calls: Vec<(&'static str, bool, u32)>,
        /// `pc.mount`, `pc.unmount` and `pc.mount_bonus`, with their three numbers.
        mount_calls: Vec<(&'static str, u32, i32, i32)>,
        /// What `attr67_vnum` answers: the window's vnum, 0 with none.
        attr67_vnum: u32,
        /// The `attr67_add_rare` calls.
        attr67_rares: u32,
        /// The `attr67_collect` calls.
        attr67_collects: u32,
        /// What `attr67_collect` answers: whether the item moved.
        attr67_moved: bool,
        rank_points: Vec<(u8, i32)>,
        rank_reward: bool,
        npc_pc: u32,
        npc_kinds: std::collections::HashMap<u32, (bool, u8)>,
        /// What `horse_name` answers, which `set_horse_name` rewrites.
        horse_names: Vec<u8>,
        /// Whether the character rides a mount.
        mounted: bool,
        /// Whether the character stands in a party.
        party: bool,
        /// Whether the character leads its party.
        party_leader: bool,
        /// The Channel the script runs on.
        channel: u8,
        /// The party flags `party_flag` reads.
        party_flags: BTreeMap<Vec<u8>, i32>,
        /// What `party_leader_pid` answers.
        leader_pid: i32,
        /// What `party_member_pids` answers.
        member_pids: Vec<u32>,
        /// What `party_near_count` answers.
        near_count: u32,
        /// The PIDs `begin_other_pc` switches to; any other PID is offline.
        online: Vec<u32>,
        /// Whether the last `begin_other_pc` switched.
        switched: bool,
        /// Whether a dungeon lives under any map index.
        dungeon: bool,
        /// Every `dungeon_new_jump_party` call, in order.
        dungeon_jumps: Vec<(i32, i32, i32)>,
        /// What `dungeon_map_index` answers.
        dungeon_index: i32,
        /// The flags `dungeon_set_flag` set, which `dungeon_get_flag` reads.
        dungeon_flags: BTreeMap<Vec<u8>, i32>,
        /// The flags `dungeon_flag_from_index` answers, by (index, name).
        dungeon_indexed: BTreeMap<(u32, Vec<u8>), i32>,
        /// Every `dungeon_notice` text, in order.
        dungeon_notices: Vec<Vec<u8>>,
        /// Every `dungeon_set_warp_location` call, in order.
        dungeon_warps: Vec<(i32, i32, i32)>,
        /// Every `dungeon_exit_all` call.
        dungeon_exits: u32,
        /// Every `dungeon_spawn_mob` call, in order.
        dungeon_spawns: Vec<(u32, i32, i32)>,
        /// Every `dungeon_new_jump` call, in order.
        dungeon_singles: Vec<(i32, i32, i32)>,
        /// Every `dungeon_clear_regen` call.
        dungeon_clears: u32,
        /// Every `dungeon_kill_all` call.
        dungeon_kills: u32,
        /// Every `dungeon_set_damage_from_race` call, in order.
        dungeon_damage_race: Vec<(u32, u32)>,
        /// Every `dungeon_set_damage_from_affect` call, in order.
        dungeon_damage_affect: Vec<(u32, u32)>,
        /// Every `set_event_flag` call, in order.
        event_sets: Vec<(Vec<u8>, i32)>,
        /// Every `set_server_timer` call, in order.
        server_timers: Vec<(Vec<u8>, f64, u32)>,
        /// Every `clear_server_timer` call, in order.
        server_clears: Vec<(Vec<u8>, u32)>,
        /// Whether `is_ds_qualified` answers true.
        ds_qualified: bool,
        /// Every `give_ds_qualification` call.
        ds_gives: u32,
        /// Every `open_ds_refine` call that found the qualification.
        ds_refines: u32,
        /// Every `open_ds_attr_change` call.
        ds_attrs: u32,
    }

    impl Fake {
        fn at_ox_manager() -> Fake {
            Fake {
                races: [(VID, OX_MANAGER)].into_iter().collect(),
                ..Fake::default()
            }
        }

        fn take(&mut self) -> Vec<(u8, String)> {
            self.scripts
                .drain(..)
                .map(|(skin, script)| (skin, String::from_utf8_lossy(&script).into_owned()))
                .collect()
        }
    }

    impl Game for Fake {
        fn level(&self) -> u8 {
            self.level
        }

        fn playtime(&self) -> i32 {
            self.playtime
        }

        fn item_vnum(&self, name: &[u8]) -> Option<u32> {
            self.item_vnums.get(name).copied()
        }

        fn give_item(&mut self, vnum: u32, count: u16) -> u32 {
            self.gives.push((vnum, count));
            self.given
        }

        fn count_item(&self, vnum: u32) -> i32 {
            self.counts.get(&vnum).copied().unwrap_or(0)
        }

        fn remove_item(&mut self, vnum: u32, count: u32) {
            self.removals.push((vnum, count));
        }

        fn skill_group(&self) -> u8 {
            self.skill_group
        }

        fn job(&self) -> u8 {
            self.job
        }

        fn held_vnum(&self, id: u32) -> Option<u32> {
            self.held.get(&id).copied()
        }
        fn held_cell(&self, id: u32) -> Option<u16> {
            self.held_cells.get(&id).copied()
        }

        fn inventory_at(&self, cell: u32) -> Option<u32> {
            self.inventory.get(&cell).copied()
        }

        fn set_socket(&mut self, id: u32, index: usize, value: i32) {
            self.sockets.push((id, index, value));
        }

        fn race_of(&self, vid: u32) -> Option<u32> {
            self.races.get(&vid).copied()
        }

        fn item_name(&self, vnum: u32) -> Option<Vec<u8>> {
            (vnum == 11_901).then(|| b"Smoking".to_vec())
        }

        fn mob_name(&self, vnum: u32) -> Option<Vec<u8>> {
            (vnum == OX_MANAGER).then(|| b"Uriel".to_vec())
        }

        fn chat(&mut self, kind: ChatKind, text: &[u8]) {
            self.chats.push((kind, text.to_vec()));
        }

        fn script(&mut self, skin: u8, script: &[u8]) {
            self.scripts.push((skin, script.to_vec()));
        }

        fn global_time(&self) -> u32 {
            1_790_000_000
        }

        fn number(&mut self, from: i32, to: i32) -> i32 {
            self.draws.push((from, to));
            to
        }
        fn ore_refine(&mut self, ore: u32, cost: i32, pct: i32, stone: Option<u32>) -> bool {
            self.melts.push((ore, cost, pct, stone));
            self.melted
        }

        fn learn_grand_master_skill(&mut self, vnum: u32, _total: i32) -> bool {
            !self.learned.insert(vnum)
        }

        fn skill_level(&self, vnum: u32) -> u8 {
            self.levels
                .get(&vnum)
                .copied()
                .unwrap_or(u8::from(self.learned.contains(&vnum)) * 40)
        }

        fn set_skill_level(&mut self, vnum: u32, level: u8) {
            self.levels.insert(vnum, level);
            self.set_levels.push((vnum, level));
        }

        fn real_alignment(&self) -> i32 {
            self.alignment
        }

        fn change_alignment(&mut self, amount: i32) {
            self.alignment_changes.push(amount);
            self.alignment = self
                .alignment
                .saturating_add(amount)
                .clamp(-200_000, 200_000);
        }

        fn has_skill_book_no_delay(&self) -> bool {
            self.skill_book_no_delay
        }

        fn remove_skill_book_no_delay(&mut self) {
            self.skill_book_removals += 1;
            self.skill_book_no_delay = false;
        }

        fn send_block_mode(&mut self, mode: u8) {
            self.blocked.push(mode);
        }

        fn map_index(&self) -> i32 {
            self.map_index
        }

        fn war_map(&self) -> i32 {
            self.war_map
        }
        fn gold(&self) -> u64 {
            self.gold
        }
        fn change_gold(&mut self, amount: i64) {
            self.gold_changes.push(amount);
        }
        fn hide_costume(&mut self, part: u8, hide: bool) {
            self.hides.push((part, hide));
        }
        fn open_sash(&mut self, combination: bool) {
            self.sash_opens.push(combination);
        }
        fn open_changelook(&mut self) {
            self.changelooks += 1;
        }
        fn mining(&mut self, npc_vid: u32) {
            self.minings.push(npc_vid);
        }
        fn refine_tool(&mut self, cell: u8, rod: bool) -> u8 {
            self.tool_refines.push((cell, rod));
            2
        }

        fn horse(&self) -> HorseView {
            self.horse
        }

        fn set_horse_level(&mut self, level: i32) {
            self.horse_levels.push(level);
            self.horse.level = u8::try_from(level).unwrap_or(0);
        }

        fn feed_horse(&mut self) {
            self.horse_feeds += 1;
        }

        fn summon_horse(&mut self, from_far: bool, vnum: u32) {
            self.horse_calls.push(("summon", from_far, vnum));
        }

        fn unsummon_horse(&mut self) {
            self.horse_calls.push(("unsummon", false, 0));
        }

        fn revive_horse(&mut self) {
            self.horse_calls.push(("revive", false, 0));
        }

        fn ride_horse(&mut self) {
            self.horse_calls.push(("ride", false, 0));
        }

        fn unride_horse(&mut self) {
            self.horse_calls.push(("unride", false, 0));
        }

        fn horse_name(&self) -> Vec<u8> {
            self.horse_names.clone()
        }

        fn mount(&mut self, vnum: u32, seconds: i32) {
            self.mount_calls.push(("mount", vnum, seconds, 0));
        }

        fn unmount(&mut self) {
            self.mount_calls.push(("unmount", 0, 0, 0));
        }

        fn mount_bonus(&mut self, apply: u8, value: i32, seconds: i32) {
            self.mount_calls
                .push(("mount_bonus", u32::from(apply), value, seconds));
        }

        fn attr67_vnum(&self) -> u32 {
            self.attr67_vnum
        }

        fn attr67_add_rare(&mut self) {
            self.attr67_rares += 1;
        }

        fn attr67_collect(&mut self) -> bool {
            self.attr67_collects += 1;
            self.attr67_moved
        }

        fn global_rank_add_point(&mut self, kind: u8, count: i32) {
            self.rank_points.push((kind, count));
        }

        fn global_rank_get_reward(&mut self) -> bool {
            std::mem::take(&mut self.rank_reward)
        }

        fn npc_is_pc(&self, vid: u32) -> bool {
            vid == self.npc_pc
        }

        fn npc_stone_and_rank(&self, vid: u32) -> Option<(bool, u8)> {
            self.npc_kinds.get(&vid).copied()
        }

        fn in_dungeon(&self) -> bool {
            false
        }

        fn set_horse_name(&mut self, name: Vec<u8>) -> u8 {
            self.horse_names = name;
            self.horse_calls.push(("set_name", false, 0));
            2
        }

        fn empire(&self) -> u8 {
            self.empire
        }

        fn hp(&self) -> i32 {
            self.hp
        }

        fn max_hp(&self) -> i32 {
            self.max_hp
        }

        fn sp(&self) -> i32 {
            self.sp
        }

        fn max_sp(&self) -> i32 {
            self.max_sp
        }

        fn exp(&self) -> u32 {
            self.exp
        }

        fn next_exp(&self) -> u32 {
            self.next_exp
        }

        fn race(&self) -> u16 {
            self.race
        }

        fn char_name(&self) -> Vec<u8> {
            self.char_name.clone()
        }

        fn char_vid(&self) -> u32 {
            self.char_vid
        }

        fn leadership(&self) -> u8 {
            self.leadership
        }

        fn position(&self) -> (i32, i32, i32, i32) {
            self.position
        }

        fn worn_vnum(&self, cell: u8) -> Option<u32> {
            self.worn.get(&cell).copied()
        }

        fn empire_of(&self, vid: u32) -> Option<u8> {
            self.empires.get(&vid).copied()
        }
        fn npc_guild(&self, vid: u32) -> u32 {
            self.npc_guilds.get(&vid).copied().unwrap_or(0)
        }
        fn land_id_at(&self, map: i32, x: i32, y: i32) -> u32 {
            self.lands_at.get(&(map, x, y)).copied().unwrap_or(0)
        }
        fn land_info(&self, land: u32) -> Option<(u32, u32, u8)> {
            self.land_infos.get(&land).copied()
        }
        fn guild_has_land(&self, guild: u32) -> bool {
            self.land_guilds.contains(&guild)
        }
        fn set_land_owner(&mut self, land: u32, guild: u32) {
            self.set_owners.push((land, guild));
        }
        fn reconstruct_object(&mut self, npc_vid: u32, vnum: u32) {
            self.reconstructs.push((npc_vid, vnum));
        }
        fn meley_register(&mut self) -> (i32, i32) {
            self.meley_registers += 1;
            self.meley_registered
        }
        fn meley_is_registered(&self) -> (bool, u8) {
            self.meley_registered_flag
        }
        fn meley_enter(&mut self) -> (bool, i32) {
            self.meley_enters += 1;
            self.meley_entered
        }
        fn is_meley_map(&self) -> bool {
            self.meley_map
        }
        fn meley_check(&mut self) {
            self.meley_checks += 1;
        }
        fn meley_leave(&mut self) {
            self.meley_leaves += 1;
        }
        fn meley_can_reward(&self) -> bool {
            self.meley_rewardable
        }
        fn meley_reward(&mut self, arg: u8) {
            self.meley_rewards.push(arg);
        }
        fn ship_create(&mut self) {
            self.ship_creates += 1;
        }
        fn ship_start(&mut self) {
            self.ship_starts += 1;
        }
        fn ship_join(&mut self) {
            self.ship_joins += 1;
        }
        fn ship_leave(&mut self) {
            self.ship_leaves += 1;
        }
        fn ship_land(&mut self) {
            self.ship_lands += 1;
        }
        fn ship_is_created(&self) -> bool {
            self.ship_created
        }
        fn ship_is_running(&self) -> bool {
            self.ship_running
        }
        fn ship_heal(&mut self, pct: u8) {
            self.ship_heals.push(pct);
        }
        fn ship_can_join(&self) -> bool {
            self.ship_joinable
        }
        fn player_id(&self) -> u32 {
            self.player_id
        }
        fn account_table_id(&self) -> u32 {
            self.account_table_id
        }
        fn gm_level(&self) -> u8 {
            self.gm_level
        }

        fn has_empty_inventory(&self, vnum: u32) -> bool {
            self.room.get(&vnum).copied().unwrap_or(false)
        }

        fn held_socket(&self, id: u32, index: usize) -> Option<i32> {
            self.held_sockets.get(&(id, index)).copied()
        }

        fn held_value(&self, id: u32, index: usize) -> Option<i32> {
            self.held_values.get(&(id, index)).copied()
        }

        fn held_shape(&self, id: u32) -> Option<(u16, u8, u8)> {
            self.held_shapes.get(&id).copied()
        }

        fn held_refine(&self, id: u32) -> Option<(Vec<u8>, u8, u32)> {
            self.held_refines.get(&id).cloned()
        }

        fn held_refine_level(&self, id: u32) -> Option<i32> {
            self.held_levels.get(&id).copied()
        }

        fn remove_quest_item(&mut self, id: u32) {
            self.removed_quest_items.push(id);
        }
        fn guild_id(&self) -> u32 {
            self.guild_id
        }

        fn is_guild_master(&self) -> bool {
            self.is_guild_master
        }

        fn guild_known(&self, _id: u32) -> bool {
            self.guild_known
        }

        fn test_server(&self) -> bool {
            self.test_server
        }

        fn language(&self) -> u8 {
            self.language
        }

        fn notice_all(&mut self, parts: Vec<Vec<u8>>) {
            self.notices.push(parts);
        }

        fn prepare_temple_ochao(&mut self) {
            self.temple_prepares += 1;
        }

        fn quest_command(&mut self, line: Vec<u8>) {
            self.quest_commands.push(line);
        }

        fn add_ox_quiz(&mut self, level: u8, question: Vec<u8>, answer: bool) {
            self.ox_quizzes.push((level, question, answer));
        }

        fn open_gaya(&mut self) {
            self.gaya_opens += 1;
        }

        fn open_gaya_market(&mut self) {
            self.gaya_market_opens += 1;
        }

        fn open_aura_absorb(&mut self, npc_vid: u32) {
            self.aura_opens.push((0, npc_vid));
        }

        fn open_aura_growth(&mut self, npc_vid: u32) {
            self.aura_opens.push((1, npc_vid));
        }

        fn open_aura_evolve(&mut self, npc_vid: u32) {
            self.aura_opens.push((2, npc_vid));
        }

        fn start_event(&mut self, index: i32, value0: i32, value1: i32) {
            self.event_starts.push((index, value0, value1));
        }

        fn end_event(&mut self, index: i32) {
            self.event_ends.push(index);
        }

        fn find_pc_vid(&self, name: &[u8]) -> u32 {
            self.pcs.get(name).copied().unwrap_or(0)
        }

        fn pid_of(&self, vid: u32) -> Option<u32> {
            self.vids.get(&vid).copied()
        }

        fn confirm_to(&mut self, to_pid: u32, msg: &[u8], timeout: i32, request_pid: u32) {
            self.confirms
                .push((to_pid, msg.to_vec(), timeout, request_pid));
        }

        fn arm_confirm_timeout(&mut self, waiter: u32, timeout: i32) {
            self.confirm_timeouts.push((waiter, timeout));
        }

        fn destroy_guild(&mut self) {
            self.disbands += 1;
        }

        fn remove_from_guild(&mut self) {
            self.leaves += 1;
        }
        fn request_make_guild(&mut self) {
            self.make_guild_asks += 1;
        }

        fn guild_call(&mut self, name: &[u8], args: &[GuildArg]) -> GuildAnswer {
            self.guild_calls.push((name.to_vec(), args.to_vec()));
            match name {
                b"get_any_war" => GuildAnswer::Number(self.any_war),
                b"name" => GuildAnswer::Text(b"Knights".to_vec()),
                b"get_reserve_war_table" => {
                    GuildAnswer::Table(std::mem::take(&mut self.reserve_table))
                }
                _ => GuildAnswer::None,
            }
        }

        fn quest_info(&mut self, info: &QuestInfo) {
            self.infos.push(info.clone());
        }

        fn find_npc(&self, race: u32) -> u32 {
            self.npcs.get(&race).copied().unwrap_or(0)
        }

        fn create_target(&mut self, target: &Target) {
            self.created.push(target.clone());
            self.targets
                .retain(|old| old.quest != target.quest || old.name != target.name);
            self.targets.push(target.clone());
        }

        fn delete_target(&mut self, quest: u32, name: Option<&[u8]>) {
            self.deleted.push((quest, name.map(<[u8]>::to_vec)));
            self.targets
                .retain(|old| old.quest != quest || name.is_some_and(|name| old.name != name));
        }

        fn target_vid(&self, quest: u32, name: &[u8]) -> u32 {
            self.targets
                .iter()
                .find(|target| target.quest == quest && target.name == name)
                .map_or(0, |target| target.vid)
        }

        fn target_at(&self, vid: u32) -> Option<(u32, Vec<u8>)> {
            self.targets
                .iter()
                .find(|target| target.vid == vid)
                .map(|target| (target.quest, target.name.clone()))
        }

        fn warp(&mut self, x: i32, y: i32, private: i32) -> bool {
            self.warps.push((x, y, private));
            !self.hack
        }

        fn war_observe(&mut self, first: u32, second: u32) -> Option<(i32, i32, i32)> {
            self.observes.push((first, second));
            (!self.hack && self.guild_known).then_some((0, 0, 0))
        }

        fn drop_with_ownership(&mut self, vnum: u32, count: i32, secs: Option<i32>) {
            self.drops.push((vnum, count, secs));
        }

        fn conqueror_level(&self) -> u8 {
            self.conqueror_level
        }

        fn conqueror_exp(&self) -> u32 {
            self.conqueror_exp
        }

        fn warp_to_village(&mut self) {
            self.villages += 1;
        }

        fn can_warp(&self) -> bool {
            self.able
        }

        fn set_conqueror(&mut self) {
            self.conquerors += 1;
        }

        fn is_mount(&self) -> bool {
            self.mounted
        }

        fn is_party(&self) -> bool {
            self.party
        }

        fn is_party_leader(&self) -> bool {
            self.party_leader
        }

        fn channel_id(&self) -> u8 {
            self.channel
        }

        fn party_flag(&self, name: &[u8]) -> i32 {
            self.party_flags.get(name).copied().unwrap_or(0)
        }

        fn set_party_flag(&mut self, name: Vec<u8>, value: i32) {
            self.party_flags.insert(name, value);
        }

        fn party_leader_pid(&self) -> i32 {
            self.leader_pid
        }

        fn party_member_pids(&self) -> Vec<u32> {
            self.member_pids.clone()
        }

        fn party_near_count(&self) -> u32 {
            self.near_count
        }

        fn begin_other_pc(&mut self, pid: u32) {
            self.switched = self.online.contains(&pid);
        }

        fn switched_pc(&self) -> bool {
            self.switched
        }

        fn end_other_pc(&mut self) {}

        fn dungeon_lives(&self, _index: u32) -> bool {
            self.dungeon
        }
        fn dungeon_new_jump_party(&mut self, map: i32, x: i32, y: i32) {
            self.dungeon_jumps.push((map, x, y));
        }
        fn dungeon_map_index(&self) -> i32 {
            self.dungeon_index
        }
        fn dungeon_set_flag(&mut self, name: Vec<u8>, value: i32) {
            self.dungeon_flags.insert(name, value);
        }
        fn dungeon_get_flag(&self, name: &[u8]) -> i32 {
            self.dungeon_flags.get(name).copied().unwrap_or(0)
        }
        fn dungeon_flag_from_index(&self, name: &[u8], index: u32) -> i32 {
            self.dungeon_indexed
                .get(&(index, name.to_vec()))
                .copied()
                .unwrap_or(0)
        }
        fn dungeon_notice(&mut self, text: Vec<u8>) {
            self.dungeon_notices.push(text);
        }
        fn dungeon_set_warp_location(&mut self, map: i32, x: i32, y: i32) {
            self.dungeon_warps.push((map, x, y));
        }
        fn dungeon_exit_all(&mut self) {
            self.dungeon_exits += 1;
        }
        fn dungeon_spawn_mob(&mut self, vnum: u32, x: i32, y: i32) {
            self.dungeon_spawns.push((vnum, x, y));
        }
        fn dungeon_new_jump(&mut self, map: i32, x: i32, y: i32) {
            self.dungeon_singles.push((map, x, y));
        }
        fn dungeon_clear_regen(&mut self) {
            self.dungeon_clears += 1;
        }
        fn dungeon_kill_all(&mut self) {
            self.dungeon_kills += 1;
        }
        fn dungeon_set_damage_from_race(&mut self, flag: u32, vid: u32) {
            self.dungeon_damage_race.push((flag, vid));
        }
        fn dungeon_set_damage_from_affect(&mut self, flag: u32, vid: u32) {
            self.dungeon_damage_affect.push((flag, vid));
        }
        fn dungeon_select(&mut self, index: Option<i32>) {
            self.dungeon_index = index.unwrap_or(0);
        }
        fn set_event_flag(&mut self, name: Vec<u8>, value: i32) {
            self.event_sets.push((name, value));
        }
        fn set_server_timer(&mut self, name: Vec<u8>, secs: f64, arg: u32) {
            self.server_timers.push((name, secs, arg));
        }
        fn clear_server_timer(&mut self, name: &[u8], arg: u32) {
            self.server_clears.push((name.to_vec(), arg));
        }

        fn is_ds_qualified(&self) -> bool {
            self.ds_qualified
        }

        fn give_ds_qualification(&mut self) {
            self.ds_qualified = true;
            self.ds_gives += 1;
        }

        fn open_ds_refine(&mut self) {
            if self.ds_qualified {
                self.ds_refines += 1;
            }
        }

        fn open_ds_attr_change(&mut self) {
            self.ds_attrs += 1;
        }

        fn is_polymorphed(&self) -> bool {
            false
        }

        fn okay_global_rank(&self) -> Vec<u8> {
            Vec::new()
        }

        fn okay_round_rank(&self) -> Vec<u8> {
            Vec::new()
        }

        fn is_engaged(&self) -> bool {
            false
        }

        fn is_married(&self) -> bool {
            false
        }

        fn change_empire(&mut self, _empire: u8) -> i32 {
            999
        }

        fn change_empire_count(&self) -> u32 {
            0
        }

        fn char_log(&mut self, _what: u32, _how: &[u8], _hint: &[u8]) {}

        fn sys_err(&mut self, _quest: &[u8], _text: &[u8]) {}
        fn empty_inventory_count(&self) -> u32 {
            self.empty_cells
        }
        fn is_trade0(&self) -> bool {
            self.trading
        }
        fn is_busy0(&self) -> bool {
            self.busy
        }
        fn give_exp2(&mut self, exp: u32) {
            self.exp2.push(exp);
        }
        fn set_skill_group(&mut self, group: u8) {
            self.skill_groups.push(group);
        }
        fn warp_local(&mut self, index: i32, x: i32, y: i32) {
            self.local_warps.push((index, x, y));
        }
        fn select_pc(&mut self, vid: u32) -> u32 {
            let old = self.char_vid;
            if let Some(pid) = self.vids.get(&vid).copied() {
                self.player_id = pid;
                self.char_vid = vid;
                old
            } else {
                0
            }
        }
        fn killee_drop_pct(&mut self, _npc_vid: u32) -> (i32, i32) {
            self.killee
        }
        fn drop_item(&mut self, vnum: u32, count: i32) {
            self.drops_plain.push((vnum, count));
        }
    }

    /// The text a Lua expression evaluates to, through the loaded libraries.
    fn text(manager: &Manager, expression: &str) -> String {
        let chunk = format!("return {expression}");
        host::load_chunk(manager.host().lua(), chunk.as_bytes(), "=test")
            .and_then(|chunk| chunk.call::<String>(()))
            .unwrap()
    }

    /// `say_title(mob_name(npc.get_race()) .. ":")` for the OX manager.
    fn title(manager: &Manager) -> String {
        format!(
            "{}Uriel:{}[ENTER]",
            text(manager, "color256(255, 230, 186)"),
            text(manager, "color256(196, 196, 196)")
        )
    }

    fn say(manager: &Manager, key: &str) -> String {
        format!(
            "{}[ENTER]",
            text(manager, &format!("translate.oxevent.{key}"))
        )
    }

    fn manager() -> &'static std::sync::Mutex<Manager> {
        static MANAGER: OnceLock<std::sync::Mutex<Manager>> = OnceLock::new();
        MANAGER.get_or_init(|| std::sync::Mutex::new(load()))
    }

    #[test]
    fn the_events_are_legacys_in_name_order() {
        let mut sorted = EVENTS;
        sorted.sort_unstable();
        assert_eq!(sorted, EVENTS);
        let set: BTreeSet<&str> = EVENTS.into_iter().collect();
        assert_eq!(set.len(), 26);
    }

    #[test]
    fn the_ox_manager_offers_its_menu_and_runs_the_entry() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake::at_ox_manager();
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        assert_eq!(
            game.take(),
            [(1, "[QUESTION 1;OX Contest |2;Inchide]".to_owned())]
        );
        assert!(manager.is_running(PID));
        assert_eq!(
            manager.quest_flag(PID, b"QUEST_CHAT_TEMP_QUEST.__status"),
            Some(0)
        );
        // A click while the menu is open is not a quest's: the shop would open.
        assert!(!manager.click(PID, VID, OX_MANAGER, &mut game));
        assert!(game.take().is_empty());
        // Continuing a `wait` that is not there is refused, and the menu stays.
        manager.answer(PID, 254, &mut game);
        assert!(game.take().is_empty());
        assert!(manager.is_running(PID));
        manager.answer(PID, 0, &mut game);
        let first = format!("{}{}[NEXT]", title(&manager), say(&manager, "_20_say"));
        assert_eq!(game.take(), [(1, first)]);
        assert_eq!(
            manager.quest_flag(PID, b"oxevent_manager.__status"),
            Some(0)
        );
        // Picking from a menu that is not there is refused, 250 included: only an answer above it
        // continues a `wait`.
        manager.answer(PID, 0, &mut game);
        manager.answer(PID, 250, &mut game);
        assert!(game.take().is_empty());
        manager.answer(PID, 251, &mut game);
        let last = format!("{}{}[DONE]", title(&manager), say(&manager, "_30_say"));
        assert_eq!(game.take(), [(1, last)]);
        assert!(!manager.is_running(PID));
        assert!(game.chats.is_empty());
        manager.disconnect(PID);
    }

    #[test]
    fn closing_the_menu_closes_the_window() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake::at_ox_manager();
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        game.take();
        // `Inchide` is entry 1, past the one quest entry.
        manager.answer(PID, 1, &mut game);
        assert_eq!(game.take(), [(0, "[DONE]".to_owned())]);
        assert!(!manager.is_running(PID));
        manager.disconnect(PID);
    }

    /// `NPC::HandleEvent` starts nothing for a character whose script waits
    /// (`questnpc.cpp:484`); the guild land seller's click, which starts its script, is the
    /// positive control.
    #[test]
    fn a_click_while_a_script_waits_starts_no_other() {
        const LAND_SELLER: u32 = 20_040;
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake::at_ox_manager();
        assert!(!manager.npcs[&LAND_SELLER].has_chat());
        assert!(manager.click(PID, VID, LAND_SELLER, &mut game));
        assert!(!game.take().is_empty());
        manager.disconnect(PID);
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        game.take();
        assert!(!manager.click(PID, VID, LAND_SELLER, &mut game));
        assert!(game.take().is_empty());
        assert!(manager.is_running(PID));
        manager.disconnect(PID);
    }

    /// A used quest item runs its own `use` scripts (`NPC::OnUseItem`): the 71000 book asks
    /// for the skill group first, and a character with none hears it has no apprenticeship.
    /// A vnum with no script runs nothing, and a waiting script starts no other.
    #[test]
    fn a_used_quest_item_runs_its_use_script() {
        const RESET_BOOK: u32 = 71_000;
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake::default();
        assert!(manager.use_item(PID, (5, RESET_BOOK), &mut game));
        let said = game.take();
        assert_eq!(said.len(), 1);
        assert!(said[0]
            .1
            .ends_with("[ENTER]You didn't start an apprenticeship yet.[ENTER][DONE]"));
        assert!(!manager.is_running(PID));
        assert!(!manager.use_item(PID, (6, 27_001), &mut game));
        assert!(game.take().is_empty());
        // A group and no boost or ward skill: the other early answer.
        game.skill_group = 1;
        assert!(manager.use_item(PID, (5, RESET_BOOK), &mut game));
        assert!(game.take()[0]
            .1
            .contains("You didn't learn any boost or ward skill yet."));
        // A learned boost skill reaches the confirmation, which waits.
        game.learned.insert(236);
        assert!(manager.use_item(PID, (5, RESET_BOOK), &mut game));
        assert!(game.take()[0].1.contains("[QUESTION"));
        assert!(manager.is_running(PID));
        assert!(!manager.use_item(PID, (5, RESET_BOOK), &mut game));
        assert!(game.take().is_empty());
        manager.disconnect(PID);
    }

    /// `CQuestManager::SIGUse` (`G/questmanager.cpp:863-886`) on a group with no
    /// `sig_use` script runs nothing and says nothing: no owner quest of the 51
    /// has one (a whole-tree `command grep -a "sig_use"` over the quest Lua finds
    /// nothing), so the dispatch is proved by the miss, as `use_item`'s own miss is.
    #[test]
    fn a_sig_use_with_no_script_runs_nothing() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake::default();
        assert!(!manager.sig_use(PID, (5, 50011), &mut game));
        assert!(game.take().is_empty());
        assert!(!manager.is_running(PID));
        manager.disconnect(PID);
    }

    /// A given ore runs the refiner's `take` scripts (`NPC::OnTakeItem`): the diamond's guard
    /// (`GetMyRefineNum(20060) == 50601`) holds, so the melt script says its line. The Fake
    /// holds fewer than 100, so the script ends instead of asking. A sword matches no guard:
    /// the script runs and says nothing. A race with no `take` script runs nothing.
    #[test]
    fn a_given_ore_runs_its_take_script() {
        const REFINER: u32 = 20_060;
        const DIAMOND_ORE: u32 = 50_601;
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake::default();
        game.races.insert(VID, REFINER);
        game.held.insert(5, DIAMOND_ORE);
        manager.set_quest_npc(VID);
        assert!(manager.take(PID, (5, REFINER), &mut game));
        let said = game.take();
        assert_eq!(said.len(), 1);
        assert!(said[0].1.contains("100 de pietricele"), "{}", said[0].1);
        assert!(!manager.is_running(PID));
        manager.disconnect(PID);
        game.held.insert(5, 19);
        assert!(manager.take(PID, (5, REFINER), &mut game));
        assert!(game.take().is_empty());
        manager.disconnect(PID);
        assert!(!manager.take(PID, (5, 9009), &mut game));
        assert!(game.take().is_empty());
        manager.disconnect(PID);
    }
    /// `item.select` moves the quest item pointer to a held ID and `item.select_cell` to the
    /// item in a base cell (`(DWORD)` of a number, so -1 finds nothing); a non-number or
    /// an unheld one moves nothing, and the call answers whether it moved.
    /// `item.get_id` reads the pointer, `item.get_cell` its held cell (0 with none or one
    /// no longer held).
    #[test]
    fn the_item_pointer_selects_by_id_and_by_cell() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake {
            held: [(5, 50_601), (9, 19)].into_iter().collect(),
            held_cells: [(5, 3), (9, 7)].into_iter().collect(),
            inventory: [(3, 5), (7, 9)].into_iter().collect(),
            ..Fake::default()
        };
        let run = |manager: &mut Manager, game: &mut Fake, source: &[u8]| {
            let code = host::load_chunk(manager.host.lua(), source, "=test");
            manager.operate(PID, game, |run, pc, _| {
                run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
            });
            game.take()
        };
        let said = run(
            &mut manager,
            &mut game,
            b"say(item.get_id()) say(item.get_cell()) \
              say(tostring(item.select(5))) say(item.get_id()) say(item.get_cell()) \
              say(tostring(item.select(77))) say(item.get_id()) \
              say(tostring(item.select('x'))) say(item.get_id()) \
              say(tostring(item.select_cell(7))) say(item.get_id()) say(item.get_cell()) \
              say(tostring(item.select_cell(4294967295))) say(item.get_id()) \
              say(tostring(item.select_cell(-1))) say(item.get_id()) \
              say(tostring(item.select_cell('x'))) say(item.get_id())",
        );
        assert_eq!(
            said,
            [(
                1,
                "0[ENTER]0[ENTER]true[ENTER]5[ENTER]3[ENTER]false[ENTER]5[ENTER]\
                 false[ENTER]5[ENTER]true[ENTER]9[ENTER]7[ENTER]false[ENTER]9[ENTER]\
                 false[ENTER]9[ENTER]false[ENTER]9[ENTER][DONE]"
                    .to_owned()
            )]
        );
        game.held.remove(&9);
        game.held_cells.remove(&9);
        let said = run(
            &mut manager,
            &mut game,
            b"say(item.get_cell()) say(item.get_vnum())",
        );
        assert_eq!(said, [(1, "0[ENTER]0[ENTER][DONE]".to_owned())]);
        manager.disconnect(PID);
    }

    /// `pc.diamond_refine` melts the quest item for `(int)` fee and odds with no stone; a
    /// non-number fee or no quest item is refused. `pc.ore_refine` takes the stone from
    /// the `(int)` third cell, 0 of a non-number, and melts with its held ID; an empty
    /// cell is refused without a melt.
    #[test]
    fn the_melt_names_melt_the_quest_item() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake {
            held: [(5, 50_601)].into_iter().collect(),
            inventory: [(4, 11)].into_iter().collect(),
            melted: true,
            ..Fake::default()
        };
        let run = |manager: &mut Manager, game: &mut Fake, source: &[u8]| {
            let code = host::load_chunk(manager.host.lua(), source, "=test");
            manager.operate(PID, game, |run, pc, _| {
                run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
            });
            game.take()
        };
        let said = run(
            &mut manager,
            &mut game,
            b"say(tostring(pc.diamond_refine(10000, 60))) \
              say(tostring(pc.diamond_refine('x', 60))) \
              item.select(5) \
              say(tostring(pc.diamond_refine(10000.9, '60'))) \
              say(tostring(pc.diamond_refine(10000))) \
              say(tostring(pc.ore_refine(3000, 60, 4))) \
              say(tostring(pc.ore_refine(3000, 60, 9))) \
              say(tostring(pc.ore_refine(3000, 60, 'x')))",
        );
        assert_eq!(
            said,
            [(
                1,
                "false[ENTER]false[ENTER]true[ENTER]true[ENTER]true[ENTER]\
                 false[ENTER]false[ENTER][DONE]"
                    .to_owned()
            )]
        );
        assert_eq!(
            game.melts,
            [
                (5, 10_000, 60, None),
                (5, 10_000, 0, None),
                (5, 3_000, 60, Some(11))
            ]
        );
        manager.disconnect(PID);
    }

    /// The boost book (50514) offers the job's boost skill of the group through `select_table`,
    /// and its "Yes" removes the book and sets the skill to 1; the reset book (71000) then
    /// sets it back to 0 and removes itself (`7and8th_skills.lua`).
    #[test]
    fn the_boost_book_teaches_the_chosen_skill_and_the_reset_book_forgets_it() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake {
            job: 3,
            skill_group: 2,
            held: [(5, 50_514), (6, 71_000)].into_iter().collect(),
            ..Fake::default()
        };
        assert!(manager.use_item(PID, (5, 50_514), &mut game));
        let said = game.take();
        assert!(said[0]
            .1
            .ends_with("[QUESTION 1;Summon Lightning Boost|2;Cancel]"));
        manager.answer(PID, 0, &mut game);
        let said = game.take();
        assert!(said[0]
            .1
            .contains("You choose to learn Summon Lightning Boost.[ENTER]"));
        assert!(game.set_levels.is_empty());
        manager.answer(PID, 0, &mut game);
        assert_eq!(game.removals, [(50_514, 1)]);
        assert_eq!(game.set_levels, [(243, 1)]);
        assert!(!manager.is_running(PID));

        assert!(manager.use_item(PID, (6, 71_000), &mut game));
        let said = game.take();
        assert!(said[said.len() - 1].1.contains("[QUESTION 1;Yes|2;No]"));
        manager.answer(PID, 0, &mut game);
        assert_eq!(game.set_levels, [(243, 1), (243, 0)]);
        assert_eq!(game.removals, [(50_514, 1), (71_000, 1)]);
        manager.disconnect(PID);
    }

    /// `pc.give_item2_select` gives as `pc.give_item2` reads, pushes nothing, and makes the item
    /// it gave the quest item, which a later run still has; a give of nothing keeps the old one.
    /// `item.set_socket` writes the quest item's socket only with two number-like arguments and
    /// an index of 0 to 5, `(int)` of each.
    #[test]
    fn the_selected_give_becomes_the_quest_item_whose_socket_is_set() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake {
            given: 77,
            held: [(77, 85_005)].into_iter().collect(),
            ..Fake::default()
        };
        let run = |manager: &mut Manager, game: &mut Fake, source: &[u8]| {
            let code = host::load_chunk(manager.host.lua(), source, "=test");
            manager.operate(PID, game, |run, pc, _| {
                run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
            });
            game.take()
        };
        let said = run(
            &mut manager,
            &mut game,
            b"item.set_socket(1, 1) \
              say(pcall(type, pc.give_item2_select(85005, 1)) and 'one' or 'none') \
              item.set_socket(1, 1) item.set_socket('2', 3.9) item.set_socket(-0.5, 8) \
              item.set_socket(6, 1) item.set_socket(-1, 1) item.set_socket(1) \
              item.set_socket(1, 'x') item.set_socket(5, -2147483649) say(item.get_vnum())",
        );
        assert_eq!(said, [(1, "none[ENTER]85005[ENTER][DONE]".to_owned())]);
        assert_eq!(game.gives, [(85_005, 1)]);
        assert_eq!(
            game.sockets,
            [(77, 1, 1), (77, 2, 3), (77, 0, 8), (77, 5, i32::MIN)]
        );
        game.given = 0;
        let said = run(
            &mut manager,
            &mut game,
            b"pc.give_item2_select(85005, 1) item.set_socket(4, 9) say(item.get_vnum())",
        );
        assert_eq!(said, [(1, "85005[ENTER][DONE]".to_owned())]);
        assert_eq!(game.sockets.last(), Some(&(77, 4, 9)));
        manager.disconnect(PID);
    }

    /// `pc.get_job` answers the job, and `pc.set_skill_level` passes `(DWORD)` and `(BYTE)` of
    /// its numbers, 0 for a missing or non-number one, and pushes nothing.
    #[test]
    fn the_job_reads_and_the_skill_level_sets_its_numbers() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake {
            job: 2,
            ..Fake::default()
        };
        let source = b"say(pc.get_job()) \
            say(pcall(type, pc.set_skill_level('239', 300)) and 'one' or 'none') \
            pc.set_skill_level() pc.set_skill_level(4.9, 'x') pc.set_skill_level(141, 40.7)";
        let code = host::load_chunk(manager.host.lua(), source, "=test");
        manager.operate(PID, &mut game, |run, pc, _| {
            run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
        });
        assert_eq!(game.take(), [(1, "2[ENTER]none[ENTER][DONE]".to_owned())]);
        assert_eq!(game.set_levels, [(239, 44), (0, 0), (4, 0), (141, 40)]);
        manager.disconnect(PID);
    }
    /// `time_to_str` is `asctime(gmtime())`: the epoch, a 2026 stamp, a negative one, a
    /// text stamp, a fraction (truncated) and no argument (0). `is_test_server` answers
    /// the deployment; `find_pc_by_name` answers the named VID and 0 for a non-string or
    /// an unknown name; the guild pair push nothing and record their calls.
    #[test]
    fn the_guild_manage_names_answer() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake {
            test_server: true,
            pcs: [(b"Yankee".to_vec(), 77)].into_iter().collect(),
            ..Fake::default()
        };
        let source = b"say(time_to_str(0)) say(time_to_str(1790000000)) say(time_to_str(-1)) \
            say(time_to_str('2000000000')) say(time_to_str(1790000000.9)) say(time_to_str()) \
            say(tostring(is_test_server())) say(find_pc_by_name('Yankee')) say(find_pc_by_name(7)) \
            say(find_pc_by_name('Nobody')) \
            say(pcall(type, pc.destroy_guild()) and 'one' or 'none') \
            say(pcall(type, pc.remove_from_guild()) and 'one' or 'none')";
        let code = host::load_chunk(manager.host.lua(), source, "=test");
        manager.operate(PID, &mut game, |run, pc, _| {
            run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
        });
        let said = "Thu Jan  1 00:00:00 1970\n[ENTER]\
            Mon Sep 21 14:13:20 2026\n[ENTER]\
            Wed Dec 31 23:59:59 1969\n[ENTER]\
            Wed May 18 03:33:20 2033\n[ENTER]\
            Mon Sep 21 14:13:20 2026\n[ENTER]\
            Thu Jan  1 00:00:00 1970\n[ENTER]\
            true[ENTER]77[ENTER]0[ENTER]0[ENTER]none[ENTER]none[ENTER][DONE]";
        assert_eq!(game.take(), [(1, said.to_owned())]);
        assert_eq!((game.disbands, game.leaves), (1, 1));
        game.test_server = false;
        let code = host::load_chunk(
            manager.host.lua(),
            b"say(tostring(is_test_server()))",
            "=test",
        );
        manager.operate(PID, &mut game, |run, pc, _| {
            run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
        });
        assert_eq!(game.take(), [(1, "false[ENTER][DONE]".to_owned())]);
        manager.disconnect(PID);
    }

    /// `item.get_vnum` reads the item the last `use` named while the character holds it, and
    /// 0 before any use and once the item is gone (the `Pc::item` Defect).
    #[test]
    fn the_quest_item_is_the_used_one_while_it_is_held() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake::default();
        let source = b"say(item.get_vnum())";
        let read = |manager: &mut Manager, game: &mut Fake| {
            let code = host::load_chunk(manager.host.lua(), source, "=test");
            manager.operate(PID, game, |run, pc, _| {
                run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
            });
            game.take()
        };
        assert_eq!(
            read(&mut manager, &mut game),
            [(1, "0[ENTER][DONE]".to_owned())]
        );
        game.held.insert(9, 72_347);
        assert!(!manager.use_item(PID, (9, 50_000), &mut game));
        assert_eq!(
            read(&mut manager, &mut game),
            [(1, "72347[ENTER][DONE]".to_owned())]
        );
        game.held.clear();
        assert_eq!(
            read(&mut manager, &mut game),
            [(1, "0[ENTER][DONE]".to_owned())]
        );
        manager.disconnect(PID);
    }
    /// The quest item's name, size, refined vnum, refine level and removal: the readers
    /// answer the held rows, and the removal takes the quest item and clears the pointer
    /// (`ClearCurrentItem`), so the second read answers the empty name and 0.
    #[test]
    fn the_quest_items_name_size_refine_and_removal_answer() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake {
            held_refines: [(9, (b"Time Spiral".to_vec(), 1, 72_348))]
                .into_iter()
                .collect(),
            held_levels: [(9, 3)].into_iter().collect(),
            ..Fake::default()
        };
        let source = b"say(item.get_name()) say(item.get_size())             say(item.get_refine_vnum()) say(item.get_level()) item.remove()             say(item.get_id()) say(item.get_name()) say(item.get_level())";
        let code = host::load_chunk(manager.host.lua(), source, "=test");
        assert!(!manager.use_item(PID, (9, 50_000), &mut game));
        manager.operate(PID, &mut game, |run, pc, _| {
            run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
        });
        assert_eq!(
            game.take(),
            [(
                1,
                "Time Spiral[ENTER]1[ENTER]72348[ENTER]3[ENTER]0[ENTER][ENTER]0[ENTER][DONE]"
                    .to_owned()
            )]
        );
        assert_eq!(game.removed_quest_items, [9]);
        manager.disconnect(PID);
    }

    /// The GM gate, the room check and the quest item's readers: `pc.is_gm` answers the
    /// level-3 gate and `pc.get_gm_level` the level; `pc.enough_inventory` answers the
    /// room by vnum and false for a non-number; the socket and value read `(int)` of
    /// the index and answer 0 outside 0-5, with no quest item or no held row; the
    /// count, type and sub-type read the held row and answer 0 with none.
    #[test]
    fn the_gm_gate_the_room_and_the_quest_items_readers_answer() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake {
            gm_level: 2,
            room: [(72_300, true)].into_iter().collect(),
            held: [(9, 72_347)].into_iter().collect(),
            held_sockets: [((9, 2), -7)].into_iter().collect(),
            held_values: [((9, 3), 10)].into_iter().collect(),
            held_shapes: [(9, (4, 16, 0))].into_iter().collect(),
            ..Fake::default()
        };
        let source = b"say(pc.is_gm() and 'gm' or 'player') say(pc.get_gm_level()) \
            say(pc.enough_inventory(72300) and 'room' or 'full') \
            say(pc.enough_inventory('72300x') and 'room' or 'full') \
            say(item.get_socket(2)) say(item.get_socket(6)) say(item.get_socket('x')) \
            say(item.get_value(3)) say(item.get_value(-1)) \
            say(item.get_count()) say(item.get_type()) say(item.get_sub_type())";
        let code = host::load_chunk(manager.host.lua(), source, "=test");
        assert!(!manager.use_item(PID, (9, 50_000), &mut game));
        manager.operate(PID, &mut game, |run, pc, _| {
            run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
        });
        assert_eq!(
            game.take(),
            [(
                1,
                "player[ENTER]2[ENTER]room[ENTER]full[ENTER]-7[ENTER]0[ENTER]0[ENTER]\
                 10[ENTER]0[ENTER]4[ENTER]16[ENTER]0[ENTER][DONE]"
                    .to_owned()
            )]
        );
        game.gm_level = 3;
        let code = host::load_chunk(
            manager.host.lua(),
            b"say(pc.is_gm() and 'gm' or 'no')",
            "=test",
        );
        manager.operate(PID, &mut game, |run, pc, _| {
            run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
        });
        assert_eq!(game.take(), [(1, "gm[ENTER][DONE]".to_owned())]);
        game.held.clear();
        game.held_sockets.clear();
        game.held_values.clear();
        game.held_shapes.clear();
        let code = host::load_chunk(
            manager.host.lua(),
            b"say(item.get_socket(2)) say(item.get_value(3)) say(item.get_count()) \
              say(item.get_type())",
            "=test",
        );
        manager.operate(PID, &mut game, |run, pc, _| {
            run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
        });
        assert_eq!(
            game.take(),
            [(1, "0[ENTER]0[ENTER]0[ENTER]0[ENTER][DONE]".to_owned())]
        );
        manager.disconnect(PID);
    }

    /// `input()` sends `[INPUT]` and waits for `CG_QUEST_INPUT_STRING`, which it reads up to its
    /// first NUL (`GotoInputState`, `CQuestManager::Input`); no other answer continues it, and
    /// no input continues another wait.
    #[test]
    fn an_input_waits_for_the_text_and_nothing_else() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake::default();
        manager.input(PID, b"early", &mut game);
        assert!(game.take().is_empty());
        let source = b"say('name?') local text = input() say(string.len(text)) wait()";
        let code = host::load_chunk(manager.host.lua(), source, "=test");
        manager.operate(PID, &mut game, |run, pc, _| {
            run.execute(pc, 0, b"q".to_vec(), 0, code, Vec::new())
        });
        assert_eq!(game.take(), [(1, "name?[ENTER][INPUT]".to_owned())]);
        manager.answer(PID, 254, &mut game);
        manager.answer(PID, 0, &mut game);
        assert!(game.take().is_empty());
        manager.input(PID, b"ab\0cd", &mut game);
        assert_eq!(game.take(), [(1, "2[ENTER][NEXT]".to_owned())]);
        manager.input(PID, b"late", &mut game);
        assert!(game.take().is_empty());
        manager.answer(PID, 254, &mut game);
        assert_eq!(game.take(), [(0, "[DONE]".to_owned())]);
        assert!(!manager.is_running(PID));
        manager.disconnect(PID);
    }

    /// `quest_setstate` writes the running state in whichever step of the script calls it
    /// (`G/questlua_quest.cpp:112-146`), and `EndRunning` moves the quest when the script
    /// ends (`G/questpc.cpp:352-371`): a `set_state` before a `wait()` lands when the answer
    /// ends the script, one after it lands too, and a refusal ends the script with the state
    /// it asked for.
    #[test]
    fn a_set_state_lands_when_the_script_ends_whichever_step_asked() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake::default();
        let quest = manager.host.quest_index(b"item_attr67add").unwrap();
        let at = usize::try_from(quest - 1).unwrap();
        let talk = manager.host.quests()[at].states[&b"talk"[..]];
        assert_ne!(talk, 0, "`talk` is not the start state");
        let status: &[u8] = b"item_attr67add.__status";
        let run = |manager: &mut Manager, source: &[u8], state: i32, game: &mut Fake| {
            let code = host::load_chunk(manager.host.lua(), source, "=test");
            manager.operate(PID, game, |run, pc, _| {
                run.execute(
                    pc,
                    quest,
                    b"item_attr67add".to_vec(),
                    state,
                    code,
                    Vec::new(),
                )
            });
        };

        run(&mut manager, b"set_state('talk') wait()", 0, &mut game);
        // A bare `[NEXT]` goes in the no-window skin (`SendScript`).
        assert_eq!(game.take(), [(0, "[NEXT]".to_owned())]);
        assert_eq!(
            manager.quest_flag(PID, status),
            Some(0),
            "the script still waits"
        );
        manager.answer(PID, 254, &mut game);
        assert_eq!(game.take(), [(0, "[DONE]".to_owned())]);
        assert_eq!(
            manager.quest_flag(PID, status),
            Some(talk),
            "the state asked for before the wait"
        );

        run(&mut manager, b"wait() set_state('start')", talk, &mut game);
        game.take();
        manager.answer(PID, 254, &mut game);
        assert_eq!(
            manager.quest_flag(PID, status),
            Some(0),
            "the state asked for after the wait"
        );

        run(
            &mut manager,
            b"set_state('talk') q.set_title('x')",
            0,
            &mut game,
        );
        assert!(!manager.is_running(PID));
        assert_eq!(
            manager.quest_flag(PID, status),
            Some(talk),
            "the refusal ends the script with the state it asked for"
        );
        manager.disconnect(PID);
    }

    /// `CQuestManager::Kill` (`G/questmanager.cpp:574-611`): the no-target kill quest
    /// (`napierdalator`, `object/notarget/kill/napierdalator.start`) runs on every kill a
    /// character makes, as it does in legacy: the character has a quest record, and the
    /// quest is a miss, so it starts (`ExecuteQuestScript`, `questnpc.cpp:526-530`).
    ///
    /// The quest's APIs are not ported, so the script ends as a script that says nothing
    /// ends, and the client sees nothing: the bare `[DONE]` is swallowed (`SendScript`,
    /// `G/questmanager.cpp:1122-1130`).
    #[test]
    fn a_kill_runs_the_no_target_quest_and_the_client_sees_nothing() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        manager.load_flags(PID, []);
        let mut game = Fake::default();
        assert!(
            manager.kill(PID, 3, Some(PID), &mut game),
            "the no-target kill quest runs"
        );
        assert_eq!(game.take(), Vec::new(), "the silent end sends nothing");
    }

    #[test]
    fn an_unported_api_ends_the_script_as_an_error_did() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        manager.event_flags.insert(b"oxevent_status".to_vec(), 1);
        let enter = text(&manager, "translate.locale.monkey_dungeon.enter");
        let cancel = text(&manager, "translate.locale.cancel");
        let menu = format!(
            "{}{}[QUESTION 1;{enter}|2;{cancel}]",
            title(&manager),
            say(&manager, "_40_say")
        );
        let waited = format!("{}{}[NEXT]", title(&manager), say(&manager, "_50_say"));
        let mut game = Fake::at_ox_manager();
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        manager.answer(PID, 0, &mut game);
        game.take();
        // The status is 1: the script asks again.
        manager.answer(PID, 255, &mut game);
        assert_eq!(game.take(), [(1, menu.clone())]);
        manager.answer(PID, 0, &mut game);
        assert_eq!(game.take(), [(1, waited)]);
        // `pc.warp` is not ported: the script ends with a bare `[DONE]`, which closes the
        // window because the client answered.
        manager.answer(PID, 255, &mut game);
        assert_eq!(game.take(), [(0, "[DONE]".to_owned())]);
        assert!(!manager.is_running(PID));
        // `cancel` returns from the script, which ends it the same way.
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        manager.answer(PID, 0, &mut game);
        manager.answer(PID, 255, &mut game);
        assert_eq!(game.take().pop(), Some((1, menu)));
        manager.answer(PID, 1, &mut game);
        assert_eq!(game.take(), [(0, "[DONE]".to_owned())]);
        assert!(!manager.is_running(PID));
        manager.event_flags.remove(&b"oxevent_status"[..]);
        manager.disconnect(PID);
    }

    #[test]
    fn an_npc_without_quests_leaves_the_click_alone() {
        let mut manager = manager().lock().unwrap();
        let mut game = Fake::default();
        // The weapon seller is in `questnpc.txt` but no quest names it.
        assert!(!manager.has_npc(9001));
        assert!(!manager.click(PID, VID, 9001, &mut game));
        // A race no quest file names at all.
        assert!(!manager.click(PID, VID, 1, &mut game));
        assert!(game.scripts.is_empty());
        assert!(manager.has_npc(OX_MANAGER));
        manager.disconnect(PID);
    }

    #[test]
    fn the_npcs_come_from_questnpc_txt_notarget_and_the_numeric_directories() {
        let manager = manager().lock().unwrap();
        let ox = &manager.npcs[&OX_MANAGER];
        let oxevent = manager.host.quest_index(b"oxevent_manager").unwrap();
        let chat = &ox.own_arg["chat"][&oxevent][&0];
        assert_eq!(chat.len(), 1);
        assert!(chat[0].when.is_none());
        assert_eq!(
            chat[0].arg.as_deref(),
            Some(&b"20011/chat/oxevent_manager.start.0.arg"[..])
        );
        assert_eq!((chat[0].quest, chat[0].state), (oxevent, 0));
        assert!(ox.own.is_empty());
        // `notarget` is NPC 0.
        let notarget = &manager.npcs[&0];
        assert!(notarget.own["login"].contains_key(&oxevent));
        // `questnpc.txt` names the guild manager, whose scripts are in `guild_man1`.
        assert_eq!(manager.npc_ids[&b"guild_man1"[..]], 11000);
        assert!(manager.npcs[&11000].has_chat());
        // Every numeric directory is an NPC; no other name is.
        for vnum in numeric_dirs(&manager.host) {
            assert!(manager.has_npc(vnum), "{vnum}");
        }
        assert!(numeric_dirs(&manager.host).contains(&OX_MANAGER));
        assert!(!numeric_dirs(&manager.host).contains(&0));
        assert_eq!(manager.event_flag(b"guild_withdraw_delay"), 1);
        assert_eq!(manager.event_flag(b"guild_disband_delay"), 1);
        assert_eq!(manager.event_flag(b"oxevent_status"), 0);
    }

    #[test]
    fn questnpc_txt_reads_as_istream_did() {
        let parsed = parse_questnpc(
            b"9001 weapon_shop\r\n12 34\n13 -34\n  20011\teulduji  \n\n7 \n5 0x1\n6 +7\n\
              8 4294967296\n-5 after\n9 never",
        );
        let names: Vec<(u32, &[u8])> = parsed
            .iter()
            .map(|(vnum, name)| (*vnum, &name[..]))
            .collect();
        assert_eq!(
            names,
            [
                (9001, &b"weapon_shop"[..]),
                (20011, b"eulduji"),
                // `strtol` reads "0x1" as 0 and "4294967296" wraps to 0 in an `int`.
                (5, b"0x1"),
                (8, b"4294967296"),
            ]
        );
        // No line end at the end of the file; a number too large for `unsigned int` stops.
        assert_eq!(parse_questnpc(b"1 a\n2 b").len(), 2);
        assert!(parse_questnpc(b"4294967296 a\n2 b").is_empty());
        assert_eq!(parse_questnpc(b"4294967295 a")[0].0, u32::MAX);
        assert!(parse_questnpc(b"").is_empty());
    }

    /// `ReadQuestCategoryToDict` (`G/questmanager.cpp:320-365`, `_QR_MS_`): white space splits
    /// a line, `atoi` reads the first word, the second names the quest, a later line wins,
    /// and a name no quest has, a line without two words and an empty line are skipped.
    #[test]
    fn questcategory_txt_reads_as_the_word_split_did() {
        let indices: BTreeMap<Vec<u8>, u32> = [
            (b"alpha".to_vec(), 3),
            (b"beta".to_vec(), 0x0001_0005),
            (b"gamma".to_vec(), 9),
            (b"delta".to_vec(), 11),
        ]
        .into_iter()
        .collect();
        let parsed = parse_questcategory(
            b"1 alpha\r\n\n7\tbeta extra\n2 nobody\n   \nlonely\n-1 gamma\n4x delta\n5 alpha",
            &indices,
        );
        let want: BTreeMap<u16, u32> = [
            (3, 5),
            // The map's key is a `WORD`: the index's low half.
            (5, 7),
            // `atoi("-1")` kept unsigned.
            (9, u32::MAX),
            // `atoi` stops at the first byte that is not a digit.
            (11, 4),
        ]
        .into_iter()
        .collect();
        assert_eq!(parsed, want);
        assert!(parse_questcategory(b"", &indices).is_empty());
        // The owner's file is empty, so every quest is in category 0.
        assert!(manager().lock().unwrap().categories.is_empty());
    }

    /// `quest_set_title`, `quest_start` and `quest_done` (`G/questlua_quest.cpp:17-45`): the
    /// title is the last argument when it is text or a number; `q.start` begins only a quest
    /// whose window has no number yet, and `q.done` ends only one that has. The clock and
    /// the counter (`:57-99`) take the last argument the same way, `(int)rint` for the values.
    #[test]
    fn the_quest_window_names_follow_legacys_argument_rules() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let mut probe = Probe {
            manager: &manager,
            game: Fake::default(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let table = Value::Table(lua.create_table().unwrap());
        probe.call("q.set_title", &[table]).unwrap();
        probe.call("q.set_title", &[]).unwrap();
        assert!(
            !probe.window.send.has(SendFlags::TITLE),
            "no text, no title"
        );
        probe
            .call("q.set_title", &[string(b"first"), string(b"la\0st")])
            .unwrap();
        assert_eq!(probe.window.title, b"la");
        probe.call("q.set_title", &[Value::Number(12.5)]).unwrap();
        assert_eq!(probe.window.title, b"12.5");
        assert!(probe.window.send.has(SendFlags::TITLE));

        // The clock and the counter (`G/questlua_quest.cpp:57-99`): the last argument,
        // string-like for the names and number-like for the values (`(int)rint`, ties
        // to even), and anything else changes nothing.
        let table = Value::Table(lua.create_table().unwrap());
        probe.call("q.set_clock_name", &[table.clone()]).unwrap();
        probe.call("q.set_clock_value", &[table.clone()]).unwrap();
        probe.call("q.set_counter_name", &[table.clone()]).unwrap();
        probe.call("q.set_counter_value", &[table]).unwrap();
        assert!(
            !probe.window.send.has(SendFlags::CLOCK_NAME)
                && !probe.window.send.has(SendFlags::CLOCK_VALUE)
                && !probe.window.send.has(SendFlags::COUNTER_NAME)
                && !probe.window.send.has(SendFlags::COUNTER_VALUE),
            "no text or number, nothing set"
        );
        probe
            .call("q.set_clock_name", &[string(b"ti\0ck")])
            .unwrap();
        probe
            .call("q.set_clock_value", &[Value::Number(2.5)])
            .unwrap();
        probe
            .call("q.set_counter_name", &[Value::Number(7.0)])
            .unwrap();
        probe.call("q.set_counter_value", &[string(b"12")]).unwrap();
        assert_eq!(probe.window.clock_name, b"ti");
        assert_eq!(probe.window.clock_value, 2, "ties to even");
        assert_eq!(probe.window.counter_name, b"7");
        assert_eq!(probe.window.counter_value, 12);
        assert!(
            probe.window.send.has(SendFlags::CLOCK_NAME)
                && probe.window.send.has(SendFlags::CLOCK_VALUE)
                && probe.window.send.has(SendFlags::COUNTER_NAME)
                && probe.window.send.has(SendFlags::COUNTER_VALUE)
        );

        // The probe's quest `q` has no number, so its window has none either.
        probe.call("q.done", &[]).unwrap();
        assert!(!probe.window.send.has(SendFlags::BEGIN), "nothing to end");
        probe.call("q.start", &[]).unwrap();
        assert!(probe.window.start && probe.window.send.has(SendFlags::BEGIN));
        assert_eq!(probe.window.index, 0, "`q` names no quest");

        probe.window = Window {
            index: 40,
            ..Window::default()
        };
        probe.call("q.start", &[]).unwrap();
        assert!(
            !probe.window.start && !probe.window.send.has(SendFlags::BEGIN),
            "a numbered quest is never begun"
        );
        probe.call("q.done", &[]).unwrap();
        assert!(!probe.window.start && probe.window.send.has(SendFlags::BEGIN));
        let categories = [(40, 0x0002_0003)].into_iter().collect();
        assert_eq!(
            probe.window.info(&categories),
            Some(QuestInfo {
                index: 40,
                category: 3,
                begin: Some(false),
                title: None,
                clock_name: None,
                clock_value: None,
                counter_name: None,
                counter_value: None,
            })
        );
        assert_eq!(Window::default().info(&categories), None);
    }

    #[test]
    fn file_names_split_as_load_state_script_did() {
        assert_eq!(
            parse_file_name(b"oxevent_manager.start"),
            FileName::Own {
                quest: b"oxevent_manager",
                state: b"start"
            }
        );
        assert_eq!(
            parse_file_name(b"lonely"),
            FileName::Own {
                quest: b"lonely",
                state: b"lonely"
            }
        );
        assert_eq!(
            parse_file_name(b"q.start.12.script"),
            FileName::Arg {
                quest: b"q",
                state: b"start",
                index: 12,
                kind: b"script"
            }
        );
        assert_eq!(
            parse_file_name(b"q.start.x.when"),
            FileName::Arg {
                quest: b"q",
                state: b"start",
                index: 0,
                kind: b"when"
            }
        );
        assert_eq!(parse_file_name(b"q.start.0"), FileName::NoIndex);
        assert_eq!(parse_file_name(b"q.start.0.arg.x"), FileName::BadName);
        assert_eq!(strtol(b" -12x"), -12);
        assert_eq!(strtol(b"99999999999999999999"), i64::MAX);
        assert_eq!(strtol(b"-99999999999999999999"), i64::MIN);
        assert_eq!(strtol(b"+7"), 7);
        assert_eq!(strtol(b" +99999999999999999999"), i64::MAX);
        assert_eq!(strtol(b""), 0);
    }

    /// `NPC::Set` skips a hidden file and one whose name starts with `CVS` in any case
    /// (`questnpc.cpp:60`, `:63`).
    #[test]
    fn hidden_and_cvs_files_are_skipped() {
        assert!(is_hidden(b".q.start"));
        assert!(is_hidden(b"CVS"));
        assert!(is_hidden(b"cvs1"));
        assert!(is_hidden(b"CvSfoo.start"));
        assert!(!is_hidden(b"q.start"));
        assert!(!is_hidden(b"x.CVS"));
        assert!(!is_hidden(b"CV"));
    }

    #[test]
    fn a_chat_menu_index_is_0_to_65535() {
        assert_eq!(arg_index(0), Some(0));
        assert_eq!(arg_index(65_535), Some(65_535));
        assert_eq!(arg_index(65_536), None);
        assert_eq!(arg_index(-1), None);
    }

    /// `RegisterNPCVnum` registered a directory only as `%u` prints the number.
    #[test]
    fn a_numeric_directory_is_a_number_as_printed() {
        assert_eq!(numeric_dir(b"20011/chat/x"), Some(20_011));
        assert_eq!(numeric_dir(b"0/chat/x"), Some(0));
        assert_eq!(numeric_dir(b"020011/chat/x"), None);
        assert_eq!(numeric_dir(b"+5/chat/x"), None);
        assert_eq!(numeric_dir(b"guild_man1/chat/x"), None);
        assert_eq!(numeric_dir(b"20011"), None);
    }

    /// The game and the dialog a bridged name is called with directly.
    struct Probe<'m> {
        manager: &'m Manager,
        game: Fake,
        output: Output,
        /// The borrowed flag maps, as `operate` lends them: the count arms persist
        /// in the `PID` map.
        borrowed: BTreeMap<u32, BTreeMap<Vec<u8>, i32>>,
        /// The PID whose map the bridge reads, as `operate`'s `Cx` holds it: the
        /// block arms switch it, and it persists across calls.
        current: u32,
        /// The open blocks' pushed PIDs, as `operate`'s `Cx` holds them.
        stack: Vec<u32>,
        /// The quest window the `q.*` arms change.
        window: Window,
    }

    impl Probe<'_> {
        fn call(&mut self, name: &str, args: &[Value]) -> mlua::Result<Vec<Value>> {
            let mut cx = Cx {
                output: std::mem::take(&mut self.output),
                game: &mut self.game,
                event_flags: &self.manager.event_flags,
                npc_ids: &self.manager.npc_ids,
                quest_indices: self.manager.host.quest_indices(),
                npc_vid: VID,
                timer_arg: 0,
                selected: None,
                quest: b"q".to_vec(),
                pending_state: None,
                pending_others: Vec::new(),
                borrowed: Some(&mut self.borrowed),
                current: self.current,
                stack: std::mem::take(&mut self.stack),
                window: std::mem::take(&mut self.window),
                categories: &self.manager.categories,
                item: 0,
                confirm_wait: None,
            };
            let args: MultiValue = args.iter().cloned().collect();
            let result = api(self.manager.host.lua(), &mut cx, name.as_bytes(), &args);
            self.output = cx.output;
            self.window = cx.window;
            self.current = cx.current;
            self.stack = cx.stack;
            Ok(result?.into_iter().collect())
        }

        fn text(&mut self, name: &str, args: &[Value]) -> Vec<u8> {
            match &self.call(name, args).unwrap()[..] {
                [Value::String(text)] => text.as_bytes().to_vec(),
                other => panic!("{name} answered {other:?}"),
            }
        }
    }

    #[test]
    fn the_bridged_names_follow_legacys_argument_rules() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let table = Value::Table(lua.create_table().unwrap());
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        // Text and numbers join, each up to a zero byte; anything else is skipped.
        let parts = [
            string(b"a\0b"),
            Value::Number(1.5),
            table.clone(),
            Value::Integer(2),
        ];
        assert!(probe.call("say", &parts).unwrap().is_empty());
        probe.call("raw_script", &[string(b"[RAW]")]).unwrap();
        probe.call("raw_script", &[table.clone()]).unwrap();
        probe.call("setleftimage", &[string(b"l.tga")]).unwrap();
        probe.call("settopimage", &[Value::Nil]).unwrap();
        probe.call("settopimage", &[string(b"t.tga")]).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&probe.output.script),
            "a1.52[ENTER][RAW][LEFTIMAGE src;l.tga][TOPIMAGE src;t.tga]"
        );
        for (name, kind) in [
            ("chat", ChatKind::Talking),
            ("cmdchat", ChatKind::Command),
            ("syschat", ChatKind::Info),
            ("notice", ChatKind::Notice),
        ] {
            probe
                .call(name, &[string(b"x"), Value::Number(3.0)])
                .unwrap();
            assert_eq!(probe.game.chats.pop(), Some((kind, b"x3".to_vec())));
        }
        let npc_id = |probe: &mut Probe, args: &[Value]| probe.call("getnpcid", args).unwrap();
        assert_eq!(
            npc_id(&mut probe, &[string(b"guild_man1")]),
            number(11000.0)
        );
        assert_eq!(npc_id(&mut probe, &[]), number(0.0));
        assert_eq!(npc_id(&mut probe, &[string(b"nobody")]), number(0.0));
        assert_eq!(
            probe
                .call("number", &[string(b"3"), Value::Number(9.9)])
                .unwrap(),
            number(9.0)
        );
        assert_eq!(
            probe.call("number", &[Value::Number(3.0)]).unwrap(),
            number(0.0)
        );
        assert_eq!(probe.game.draws, [(3, 9)]);
        assert_eq!(probe.text("mob_name", &number(20011.7)), b"Uriel");
        assert_eq!(
            probe.text("mob_name", &number(20011.0 + 4_294_967_296.0)),
            b"Uriel"
        );
        assert_eq!(probe.text("mob_name", &number(-4_294_947_285.0)), b"Uriel");
        assert!(probe.text("mob_name", &number(f64::NAN)).is_empty());
        assert!(probe.text("mob_name", &number(1e30)).is_empty());
        assert!(probe.text("mob_name", &[table.clone()]).is_empty());
        assert_eq!(probe.text("item_name", &number(11901.0)), b"Smoking");
        assert_eq!(probe.text("item_name", &[string(b"11901")]), b"Smoking");
        assert!(probe.text("item_name", &number(20011.0)).is_empty());
        assert!(probe.text("item_name", &number(f64::NAN)).is_empty());
        assert!(probe.text("item_name", &[table.clone()]).is_empty());
        assert_eq!(
            probe.call("npc.get_race", &[]).unwrap(),
            number(f64::from(OX_MANAGER))
        );
        assert_eq!(
            probe.call("npc.getrace", &[]).unwrap(),
            number(f64::from(OX_MANAGER))
        );
        probe.game.races.clear();
        assert_eq!(probe.call("npc.get_race", &[]).unwrap(), number(0.0));
        let flag =
            |probe: &mut Probe, args: &[Value]| probe.call("game.get_event_flag", args).unwrap();
        assert_eq!(
            flag(&mut probe, &[string(b"guild_disband_delay")]),
            number(1.0)
        );
        assert_eq!(flag(&mut probe, &[string(b"nothing")]), number(0.0));
        assert_eq!(flag(&mut probe, &[table.clone()]), number(0.0));
        for name in ["get_time", "get_global_time"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(1_790_000_000.0));
        }
    }

    /// The quiet globals: `LC` answers the tongue, `notice_all` snapshots six parts
    /// and refuses a seventh argument, `command` runs a string and stays quiet on
    /// anything else, and `temple_ochao.initialize` prepares. None pushes.
    #[test]
    fn the_quiet_globals_snapshot_refuse_and_prepare() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        probe.game.language = 5;
        assert_eq!(probe.text("LC", &[]), b"de");
        probe.game.language = 0;
        assert_eq!(probe.text("LC", &[]), b"en");
        assert!(probe
            .call("notice_all", &[string(b"hi")])
            .unwrap()
            .is_empty());
        assert_eq!(
            probe.game.notices.pop(),
            Some(vec![
                b"hi".to_vec(),
                b"(null)".to_vec(),
                b"(null)".to_vec(),
                b"(null)".to_vec(),
                b"(null)".to_vec(),
                b"(null)".to_vec(),
            ])
        );
        let seven: Vec<Value> = (0..7).map(|_| string(b"x")).collect();
        assert!(probe.call("notice_all", &seven).unwrap().is_empty());
        assert!(probe.game.notices.is_empty());
        assert!(probe
            .call("command", &[string(b"battle_pass")])
            .unwrap()
            .is_empty());
        assert_eq!(
            probe.game.quest_commands.pop(),
            Some(b"battle_pass".to_vec())
        );
        assert!(probe.call("command", &[Value::Nil]).unwrap().is_empty());
        assert!(probe.game.quest_commands.is_empty());
        assert!(probe
            .call("temple_ochao.initialize", &[])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.temple_prepares, 1);
    }

    /// `add_ox_quiz` lands the level, question and answer and pushes the level back;
    /// a number question formats, a missing one stays quiet, and the third slot's
    /// truth is the answer.
    #[test]
    fn the_ox_quiz_lands_and_pushes_its_level() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let quiz = probe.call(
            "add_ox_quiz",
            &[
                Value::Number(1.0),
                string(b"Steel cuts?"),
                Value::Boolean(true),
            ],
        );
        assert!(
            matches!(quiz.unwrap()[..], [Value::Number(level)] if level.to_bits() == 1.0_f64.to_bits())
        );
        assert_eq!(
            probe.game.ox_quizzes.pop(),
            Some((1, b"Steel cuts?".to_vec(), true))
        );
        let quiz = probe.call(
            "add_ox_quiz",
            &[Value::Number(7.0), Value::Number(42.0), Value::Nil],
        );
        assert!(
            matches!(quiz.unwrap()[..], [Value::Number(level)] if level.to_bits() == 7.0_f64.to_bits())
        );
        assert_eq!(
            probe.game.ox_quizzes.pop(),
            Some((7, b"42".to_vec(), false))
        );
        let quiz = probe.call("add_ox_quiz", &[Value::Number(3.0)]);
        assert!(
            matches!(quiz.unwrap()[..], [Value::Number(level)] if level.to_bits() == 3.0_f64.to_bits())
        );
        assert!(probe.game.ox_quizzes.is_empty());
    }

    /// The `building.*` calls follow their argument rules: the id read pushes 0 for a
    /// non-number, the info triple answers 10^9 for a non-number or an unknown land,
    /// the owner write and the rebuild push nothing, and the owned read pushes true
    /// for a non-number.
    /// The Meley lobby pushes its constants and its judged pairs: the requirements,
    /// the limit, the sub map, the register's pair, the registered boolean with its
    /// channel, the enter's boolean with its limit, the map read, and the quiet
    /// check and leave.
    #[test]
    fn the_meley_lobby_pushes_its_pairs() {
        let manager = manager().lock().unwrap();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let bits = |values: Vec<Value>| {
            values
                .iter()
                .map(|value| match value {
                    Value::Number(number) => number.to_bits(),
                    Value::Boolean(flag) => u64::from(*flag),
                    _ => 0,
                })
                .collect::<Vec<_>>()
        };
        let pair = probe.call("MeleyLair.GetRequirments", &[]).unwrap();
        assert_eq!(bits(pair), [3.0_f64.to_bits(), 600.0_f64.to_bits()]);
        let limit = probe.call("MeleyLair.GetPartecipantsLimit", &[]).unwrap();
        assert_eq!(bits(limit), [16.0_f64.to_bits()]);
        let sub = probe.call("MeleyLair.GetSubMapIndex", &[]).unwrap();
        assert_eq!(bits(sub), [62.0_f64.to_bits()]);
        probe.game.meley_registered = (6, 300);
        let pair = probe.call("MeleyLair.Register", &[]).unwrap();
        assert_eq!(bits(pair), [6.0_f64.to_bits(), 300.0_f64.to_bits()]);
        assert_eq!(probe.game.meley_registers, 1);
        probe.game.meley_registered_flag = (true, 2);
        let pair = probe.call("MeleyLair.IsRegistered", &[]).unwrap();
        assert_eq!(pair.len(), 2);
        assert!(matches!(pair[0], Value::Boolean(true)));
        assert_eq!(bits(pair), [1, 2.0_f64.to_bits()]);
        probe.game.meley_entered = (true, 0);
        let pair = probe.call("MeleyLair.Enter", &[]).unwrap();
        assert!(matches!(pair[0], Value::Boolean(true)));
        assert_eq!(bits(pair), [1, 0.0_f64.to_bits()]);
        assert_eq!(probe.game.meley_enters, 1);
        probe.game.meley_map = true;
        let map = probe.call("MeleyLair.IsMeleyMap", &[]).unwrap();
        assert!(matches!(map[..], [Value::Boolean(true)]));
        assert!(probe.call("MeleyLair.Check", &[]).unwrap().is_empty());
        assert_eq!(probe.game.meley_checks, 1);
        assert!(probe.call("MeleyLair.Leave", &[]).unwrap().is_empty());
        assert_eq!(probe.game.meley_leaves, 1);
    }

    /// The ship lobby's answers: the four constants are true, the two state reads
    /// answer the shadow, the five warps queue pushing nothing, the heal queues its
    /// low byte (`(BYTE)` of the number) and a non-number logs and queues nothing,
    /// and `can_join` answers the shadow.
    #[test]
    fn the_ship_lobby_answers_its_constants_and_queues_its_warps() {
        let manager = manager().lock().unwrap();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        for name in [
            "ship_defense_mgr.need_party",
            "ship_defense_mgr.need_ticket",
            "ship_defense_mgr.spawn_wood_repair",
            "ship_defense_mgr.require_cooldown",
        ] {
            let answer = probe.call(name, &[]).unwrap();
            assert!(matches!(answer[..], [Value::Boolean(true)]), "{name}");
        }
        probe.game.ship_created = true;
        let answer = probe.call("ship_defense_mgr.is_created", &[]).unwrap();
        assert!(matches!(answer[..], [Value::Boolean(true)]));
        probe.game.ship_running = true;
        let answer = probe.call("ship_defense_mgr.is_running", &[]).unwrap();
        assert!(matches!(answer[..], [Value::Boolean(true)]));
        assert!(probe
            .call("ship_defense_mgr.create", &[])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.ship_creates, 1);
        assert!(probe
            .call("ship_defense_mgr.start", &[])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.ship_starts, 1);
        assert!(probe.call("ship_defense_mgr.join", &[]).unwrap().is_empty());
        assert_eq!(probe.game.ship_joins, 1);
        assert!(probe
            .call("ship_defense_mgr.leave", &[])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.ship_leaves, 1);
        assert!(probe.call("ship_defense_mgr.land", &[]).unwrap().is_empty());
        assert_eq!(probe.game.ship_lands, 1);
        let answer = probe
            .call(
                "ship_defense_mgr.set_alliance_hp_pct",
                &[Value::Number(257.0)],
            )
            .unwrap();
        assert!(answer.is_empty());
        assert_eq!(probe.game.ship_heals, [1]);
        let answer = probe
            .call(
                "ship_defense_mgr.set_alliance_hp_pct",
                &[Value::String(
                    probe.manager.host.lua().create_string(b"no").unwrap(),
                )],
            )
            .unwrap();
        assert!(answer.is_empty());
        assert_eq!(probe.game.ship_heals, [1]);
        probe.game.ship_joinable = true;
        let answer = probe.call("ship_defense_mgr.can_join", &[]).unwrap();
        assert!(matches!(answer[..], [Value::Boolean(true)]));
    }

    #[test]
    fn the_building_calls_follow_their_argument_rules() {
        let manager = manager().lock().unwrap();
        let number = |value: f64| Value::Number(value);
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        probe.game.lands_at.insert((1, 100, 200), 7);
        probe.game.land_infos.insert(7, (50_000, 9, 10));
        probe.game.land_guilds.insert(9);
        let id = probe.call(
            "building.get_land_id",
            &[number(1.0), number(100.0), number(200.0)],
        );
        assert!(
            matches!(id.unwrap()[..], [Value::Number(id)] if id.to_bits() == 7.0_f64.to_bits())
        );
        let id = probe.call(
            "building.get_land_id",
            &[number(1.0), Value::Nil, number(2.0)],
        );
        assert!(
            matches!(id.unwrap()[..], [Value::Number(id)] if id.to_bits() == 0.0_f64.to_bits())
        );
        let bits = |values: Vec<Value>| {
            values
                .iter()
                .map(|value| match value {
                    Value::Number(number) => number.to_bits(),
                    _ => 0,
                })
                .collect::<Vec<_>>()
        };
        let info = probe
            .call("building.get_land_info", &[number(7.0)])
            .unwrap();
        assert_eq!(
            bits(info),
            [
                50_000.0_f64.to_bits(),
                9.0_f64.to_bits(),
                10.0_f64.to_bits()
            ]
        );
        let info = probe
            .call("building.get_land_info", &[number(8.0)])
            .unwrap();
        assert_eq!(
            bits(info),
            [
                1_000_000_000.0_f64.to_bits(),
                1_000_000_000.0_f64.to_bits(),
                1_000_000_000.0_f64.to_bits()
            ]
        );
        let info = probe.call("building.get_land_info", &[Value::Nil]).unwrap();
        assert_eq!(
            bits(info),
            [
                1_000_000_000.0_f64.to_bits(),
                1_000_000_000.0_f64.to_bits(),
                1_000_000_000.0_f64.to_bits()
            ]
        );
        assert!(probe
            .call("building.set_land_owner", &[number(7.0), number(9.0)])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.set_owners.pop(), Some((7, 9)));
        assert!(probe
            .call("building.set_land_owner", &[number(7.0), Value::Nil])
            .unwrap()
            .is_empty());
        assert!(probe.game.set_owners.is_empty());
        let has = probe.call("building.has_land", &[number(9.0)]);
        assert!(matches!(has.unwrap()[..], [Value::Boolean(true)]));
        let has = probe.call("building.has_land", &[number(10.0)]);
        assert!(matches!(has.unwrap()[..], [Value::Boolean(false)]));
        let has = probe.call("building.has_land", &[Value::Nil]);
        assert!(matches!(has.unwrap()[..], [Value::Boolean(true)]));
        assert!(probe
            .call("building.reconstruct", &[number(14062.0)])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.reconstructs.pop(), Some((VID, 14062)));
        assert!(probe
            .call("building.reconstruct", &[Value::Nil])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.reconstructs.pop(), Some((VID, 0)));
    }

    /// The window globals push nothing: the Gaya pair opens, and the aura trio opens
    /// on the click's NPC.
    #[test]
    fn the_window_globals_open_on_the_npc() {
        let manager = manager().lock().unwrap();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        assert!(probe.call("game.open_gaya", &[]).unwrap().is_empty());
        assert!(probe.call("game.open_gaya_market", &[]).unwrap().is_empty());
        assert_eq!(
            (probe.game.gaya_opens, probe.game.gaya_market_opens),
            (1, 1)
        );
        assert!(probe
            .call("game.open_aura_absorb_window", &[])
            .unwrap()
            .is_empty());
        assert!(probe
            .call("game.open_aura_growth_window", &[])
            .unwrap()
            .is_empty());
        assert!(probe
            .call("game.open_aura_evolve_window", &[])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.aura_opens, [(0, VID), (1, VID), (2, VID)]);
    }

    /// `start_event` stages the index and its two values (a numeric string counts, a
    /// missing value reads 0) and pushes nothing; a non-number first pushes 0 and
    /// stages nothing. `end_event` stages the index the same way.
    #[test]
    fn the_event_pair_stages_or_pushes_zero() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        assert!(probe
            .call(
                "start_event",
                &[Value::Number(1.0), string(b"50"), Value::Nil]
            )
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.event_starts, [(1, 50, 0)]);
        let pushed = probe.call("start_event", &[string(b"bogus")]).unwrap();
        assert!(matches!(pushed[..], [Value::Number(zero)] if zero.to_bits() == 0.0_f64.to_bits()));
        assert_eq!(probe.game.event_starts.len(), 1);
        assert!(probe
            .call("end_event", &[Value::Number(1.0)])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.event_ends, [1]);
        let pushed = probe.call("end_event", &[]).unwrap();
        assert!(matches!(pushed[..], [Value::Number(zero)] if zero.to_bits() == 0.0_f64.to_bits()));
        assert_eq!(probe.game.event_ends.len(), 1);
    }

    /// `pc.warp` answers false without two numbers; past them it warps and answers the
    /// game, with the third number as the private index, and `pc.get_conqueror_level`
    /// reads the game.
    #[test]
    fn the_warp_and_the_conqueror_follow_legacys_argument_rules() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let table = Value::Table(lua.create_table().unwrap());
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let boolean = |value: bool| vec![Value::Boolean(value)];
        let warp = |probe: &mut Probe, args: &[Value]| probe.call("pc.warp", args).unwrap();
        assert_eq!(warp(&mut probe, &[]), boolean(false));
        assert_eq!(
            warp(&mut probe, &[Value::Number(1.0), table.clone()]),
            boolean(false)
        );
        assert_eq!(
            warp(
                &mut probe,
                &[Value::Number(853_700.7), Value::Number(1_416_400.2)],
            ),
            boolean(true)
        );
        assert_eq!(
            warp(
                &mut probe,
                &[
                    Value::Number(853_700.0),
                    Value::Number(1_416_400.0),
                    Value::Number(209_0000.0),
                ],
            ),
            boolean(true)
        );
        probe.game.hack = true;
        assert_eq!(
            warp(
                &mut probe,
                &[Value::Number(853_700.0), Value::Number(1_416_400.0)],
            ),
            boolean(false)
        );
        probe.game.hack = false;
        assert_eq!(
            probe.game.warps,
            [
                (853_700, 1_416_400, 0),
                (853_700, 1_416_400, 2_090_000),
                (853_700, 1_416_400, 0),
            ]
        );
        probe.game.conqueror_level = 33;
        assert_eq!(
            probe.call("pc.get_conqueror_level", &[]).unwrap(),
            number(33.0)
        );
        probe.game.conqueror_level = 2;
        probe.game.conqueror_exp = 0x0001_0203;
        assert_eq!(
            probe.call("pc.get_conqueror_exp", &[]).unwrap(),
            number(66_051.0)
        );
        assert_eq!(
            probe.call("pc.get_conqueror_next_exp", &[]).unwrap(),
            number(1_300_204.0),
            "the cost of conqueror level 2"
        );
    }

    /// `warp_to_village` pushes nothing, `pc.can_warp` answers the game, and
    /// `pc.set_conqueror` pushes nothing.
    #[test]
    fn the_village_the_gate_and_the_conqueror_follow_the_game() {
        let manager = manager().lock().unwrap();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let boolean = |value: bool| vec![Value::Boolean(value)];
        assert!(probe.call("warp_to_village", &[]).unwrap().is_empty());
        assert_eq!(probe.game.villages, 1);
        assert_eq!(probe.call("pc.can_warp", &[]).unwrap(), boolean(false));
        probe.game.able = true;
        assert_eq!(probe.call("pc.can_warp", &[]).unwrap(), boolean(true));
        assert!(probe.call("pc.set_conqueror", &[]).unwrap().is_empty());
        assert_eq!(probe.game.conquerors, 1);
    }

    /// `game.drop_item_with_ownership` pushes nothing; one to three arguments drops
    /// (1 with one argument, `None` seconds with no third number), any other count
    #[test]
    fn the_owned_drop_follows_legacys_argument_rules() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let table = Value::Table(lua.create_table().unwrap());
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let drop =
            |probe: &mut Probe, args: &[Value]| probe.call("game.drop_item_with_ownership", args);
        assert!(drop(&mut probe, &[]).unwrap().is_empty());
        assert!(drop(&mut probe, &[table.clone()]).unwrap().is_empty());
        assert_eq!(probe.game.drops, [(0, 1, None)]);
        drop(
            &mut probe,
            &[
                Value::Number(2145.0),
                Value::Number(3.0),
                Value::Number(300.0),
            ],
        )
        .unwrap();
        drop(&mut probe, &[Value::Number(2145.0), Value::Number(3.0)]).unwrap();
        drop(
            &mut probe,
            &[Value::Number(2145.0), Value::Number(3.0), string(b"soon")],
        )
        .unwrap();
        drop(
            &mut probe,
            &[
                Value::Number(2145.0),
                Value::Number(3.0),
                Value::Number(300.0),
                Value::Number(1.0),
            ],
        )
        .unwrap();
        drop(
            &mut probe,
            &[Value::Number(2145.0), table.clone(), Value::Number(300.0)],
        )
        .unwrap();
        assert_eq!(
            probe.game.drops,
            [
                (0, 1, None),
                (2145, 3, Some(300)),
                (2145, 3, None),
                (2145, 3, None),
                (2145, 0, Some(300)),
            ]
        );
    }

    /// The `guild.*` arms: a number-like argument arrives dworded, a text-only one arrives
    /// raw, and a war name stays unbridged. The Fake answers nothing, so every call pushes
    /// nothing back.
    #[test]
    fn the_guild_names_dword_their_numbers() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        // A fractional id arrives dworded, truncating toward zero.
        assert!(probe.call("guild.level", &number(7.9)).unwrap().is_empty());
        assert_eq!(
            probe.game.guild_calls.pop(),
            Some((b"level".to_vec(), vec![GuildArg::Number(7)]))
        );
        assert!(probe.call("guild.get_sp0", &[]).unwrap().is_empty());
        assert_eq!(
            probe.game.guild_calls.pop(),
            Some((b"get_sp0".to_vec(), Vec::new()))
        );
        // A text name arrives raw; a wrapping id arrives by its low 32 bits.
        let name = Value::String(lua.create_string(b"Knights").unwrap());
        assert!(probe
            .call("guild.change_master", &[name])
            .unwrap()
            .is_empty());
        assert_eq!(
            probe.game.guild_calls.pop(),
            Some((
                b"change_master".to_vec(),
                vec![GuildArg::Text(b"Knights".to_vec())]
            ))
        );
        assert!(probe
            .call("guild.level", &number(4_294_967_296.0 + 12.9))
            .unwrap()
            .is_empty());
        assert_eq!(
            probe.game.guild_calls.pop(),
            Some((b"level".to_vec(), vec![GuildArg::Number(12)]))
        );
        let error = probe
            .call("guild.is_war", &number(7.0))
            .unwrap_err()
            .to_string();
        assert!(error.contains("not ported: guild.is_war"), "{error}");
        // A table answer arrives as nested 1-based tables.
        probe.game.reserve_table = vec![vec![3.0, 9.0, 7.0, 2.0]];
        let pushed = probe.call("guild.get_reserve_war_table", &[]).unwrap();
        assert_eq!(pushed.len(), 1);
        let Value::Table(outer) = &pushed[0] else {
            panic!("a table, not {pushed:?}");
        };
        assert_eq!(outer.len().unwrap(), 1);
        let row: mlua::Table = outer.get(1).unwrap();
        assert_eq!(row.len().unwrap(), 4);
        let cells: Vec<f64> = vec![
            row.get(1).unwrap(),
            row.get(2).unwrap(),
            row.get(3).unwrap(),
            row.get(4).unwrap(),
        ];
        assert_eq!(cells, [3.0, 9.0, 7.0, 2.0]);
    }

    /// The `pc.*` item arms: the give, and the count under both names.
    #[test]
    fn the_pc_item_names_give_count_and_remove() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        // `pc.give_item2`: a vnum, a count rounded to even, 1 without one, nothing for a
        // count that rounds to 0, a name's vnum, and nothing for an unknown name or a table.
        probe.game.given = 0x0102_0304;
        probe.game.item_vnums.insert(b"Okey".to_vec(), 79_506);
        let text = |text: &str| Value::String(lua.create_string(text).unwrap());
        let give = |probe: &mut Probe, args: &[Value]| probe.call("pc.give_item2", args).unwrap();
        assert_eq!(
            give(&mut probe, &[Value::Number(70_101.9)]),
            number(16_909_060.0)
        );
        assert_eq!(
            give(&mut probe, &[text("85000"), Value::Number(2.5)]),
            number(16_909_060.0)
        );
        assert_eq!(
            give(&mut probe, &[Value::Number(50_033.0), Value::Number(3.5)]),
            number(16_909_060.0)
        );
        assert_eq!(
            give(&mut probe, &[Value::Number(1.0), Value::Number(0.4)]),
            number(0.0)
        );
        assert_eq!(
            give(&mut probe, &[Value::Number(2.0), Value::Number(-3.0)]),
            number(16_909_060.0)
        );
        assert_eq!(
            give(&mut probe, &[Value::Number(3.0), Value::Number(70_000.0)]),
            number(16_909_060.0)
        );
        assert_eq!(give(&mut probe, &[text("Okey")]), number(16_909_060.0));
        assert_eq!(give(&mut probe, &[text("Nothing")]), number(0.0));
        let table = Value::Table(lua.create_table().unwrap());
        assert_eq!(give(&mut probe, &[table]), number(0.0));
        assert_eq!(
            probe.game.gives,
            [
                (70_101, 1),
                (85_000, 2),
                (50_033, 4),
                (2, 1),
                (3, u16::MAX),
                (79_506, 1),
            ]
        );
        // `pc.count_item` under both names: the last argument's number, dworded, or the
        // first argument's name when the last is text; 0 for anything else.
        probe.game.counts.insert(79_506, -0x0102_0304);
        probe.game.counts.insert(27_001, 0x0a0b);
        for name in ["pc.count_item", "pc.countitem"] {
            let count = |probe: &mut Probe, args: &[Value]| probe.call(name, args).unwrap();
            assert_eq!(count(&mut probe, &[text("Okey")]), number(-16_909_060.0));
            assert_eq!(count(&mut probe, &[text("27001")]), number(2571.0));
            assert_eq!(
                count(&mut probe, &[Value::Number(27_001.7)]),
                number(2571.0)
            );
            assert_eq!(
                count(&mut probe, &[text("Okey"), Value::Number(27_001.0)]),
                number(2571.0),
                "the last argument is the number"
            );
            assert_eq!(
                count(&mut probe, &[text("Okey"), text("anything")]),
                number(-16_909_060.0),
                "text last: the first names it"
            );
            assert_eq!(count(&mut probe, &[text("Nothing")]), number(0.0));
            let table = Value::Table(lua.create_table().unwrap());
            assert_eq!(count(&mut probe, &[table.clone()]), number(0.0));
            assert_eq!(count(&mut probe, &[table, text("Okey")]), number(0.0));
            assert_eq!(count(&mut probe, &[]), number(0.0));
        }
    }

    /// `pc.remove_item` and `pc.removeitem`.
    #[test]
    fn the_pc_removal_names_take_one_or_the_count() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        probe.game.item_vnums.insert(b"Okey".to_vec(), 79_506);
        let text = |text: &str| Value::String(lua.create_string(text).unwrap());
        // `pc.remove_item` under both names pushes nothing: one argument takes 1, a second
        // is the count, dworded, or 0 when it is no number; a bad vnum or argument count
        // takes nothing.
        for name in ["pc.remove_item", "pc.removeitem"] {
            let remove = |probe: &mut Probe, args: &[Value]| probe.call(name, args).unwrap();
            assert_eq!(remove(&mut probe, &[Value::Number(85_000.0)]), []);
            assert_eq!(
                remove(&mut probe, &[text("90010"), Value::Number(10.9)]),
                []
            );
            assert_eq!(remove(&mut probe, &[text("Okey"), Value::Number(-1.0)]), []);
            assert_eq!(remove(&mut probe, &[Value::Number(5.0), text("x")]), []);
            assert_eq!(
                remove(&mut probe, &[text("Nothing"), Value::Number(1.0)]),
                []
            );
            let table = Value::Table(lua.create_table().unwrap());
            assert_eq!(remove(&mut probe, &[table, Value::Number(1.0)]), []);
            assert_eq!(remove(&mut probe, &[]), []);
            let three = [Value::Number(6.0), Value::Number(1.0), Value::Number(1.0)];
            assert_eq!(remove(&mut probe, &three), []);
        }
        assert_eq!(
            probe.game.removals,
            [
                (85_000, 1),
                (90_010, 10),
                (79_506, u32::MAX),
                (5, 0),
                (85_000, 1),
                (90_010, 10),
                (79_506, u32::MAX),
                (5, 0),
            ]
        );
    }

    /// The `pc.*` arms: the level, the minutes played under both names, the grand-master read
    /// (each numbered read bumps the count in the flags, dworded), and the skill level.
    #[test]
    fn the_pc_names_answer_and_count_the_reads() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        probe.game.level = 40;
        assert_eq!(probe.call("pc.get_level", &[]).unwrap(), number(40.0));
        probe.game.playtime = -0x0102_0304;
        assert_eq!(
            probe.call("pc.get_playtime", &[]).unwrap(),
            number(-16_909_060.0)
        );
        assert_eq!(
            probe.call("pc.getplaytime", &[]).unwrap(),
            number(-16_909_060.0)
        );
        let table = Value::Table(lua.create_table().unwrap());
        // a non-number pushes nothing.
        let climbed = |probe: &mut Probe, args: &[Value]| {
            probe.call("pc.learn_grand_master_skill", args).unwrap()
        };
        assert_eq!(climbed(&mut probe, &number(1.9)), [Value::Boolean(false)]);
        assert_eq!(climbed(&mut probe, &number(1.9)), [Value::Boolean(true)]);
        assert_eq!(
            probe.borrowed[&PID].get(b"training_grandmaster_skill.skill1".as_slice()),
            Some(&2),
            "each numbered read bumps the count, dworded"
        );
        assert!(climbed(&mut probe, &[table.clone()]).is_empty());
        assert!(climbed(&mut probe, &[]).is_empty());
        // The Fake answers 40 for a read vnum, 0 else; a non-number logs and answers 0.
        let leveled =
            |probe: &mut Probe, args: &[Value]| probe.call("pc.get_skill_level", args).unwrap();
        assert_eq!(leveled(&mut probe, &number(1.9)), number(40.0));
        assert_eq!(leveled(&mut probe, &number(2.0)), number(0.0));
        assert_eq!(leveled(&mut probe, &[table.clone()]), number(0.0));
        assert_eq!(leveled(&mut probe, &[]), number(0.0));
    }

    /// The `horse.*` names: without a horse the pools and percentages read 0; with one they
    /// read the pools and `MINMAX(0, x * 100 / max, 100)`; `set_level` sets only a number,
    /// truncated and clamped to 0..30; `advance` stops at 30; `feed` needs a living horse.
    #[test]
    fn the_horse_names_read_the_pools_and_clamp_the_level() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        probe.game.horse = HorseView {
            level: 0,
            health: 7,
            stamina: 9,
            max_health: 1,
            max_stamina: 1,
            riding: false,
            summoned: 0,
        };
        assert_eq!(probe.call("horse.get_health", &[]).unwrap(), number(0.0));
        assert_eq!(
            probe.call("horse.get_stamina_pct", &[]).unwrap(),
            number(0.0)
        );
        assert_eq!(
            probe.call("pc.is_horse_alive", &[]).unwrap(),
            [Value::Boolean(false)]
        );
        probe.game.horse = HorseView {
            level: 21,
            health: 13,
            stamina: 250,
            max_health: 40,
            max_stamina: 140,
            riding: true,
            summoned: 0,
        };
        assert_eq!(probe.call("horse.get_health", &[]).unwrap(), number(13.0));
        assert_eq!(
            probe.call("horse.get_health_pct", &[]).unwrap(),
            number(32.0)
        );
        assert_eq!(
            probe.call("horse.get_stamina_pct", &[]).unwrap(),
            number(100.0)
        );
        assert_eq!(probe.call("horse.get_grade", &[]).unwrap(), number(3.0));
        assert_eq!(probe.call("horse.is_riding", &[]).unwrap(), number(1.0));
        assert_eq!(probe.call("pc.get_horse_level", &[]).unwrap(), number(21.0));
        assert_eq!(
            probe.call("horse.is_dead", &[]).unwrap(),
            [Value::Boolean(false)]
        );
        assert!(probe.call("horse.feed", &[]).unwrap().is_empty());
        assert_eq!(probe.game.horse_feeds, 1);
        let text = Value::String(lua.create_string(b"x").unwrap());
        assert!(probe.call("horse.set_level", &[text]).unwrap().is_empty());
        assert!(probe.game.horse_levels.is_empty(), "a word sets nothing");
        probe
            .call("horse.set_level", &[Value::Number(99.7)])
            .unwrap();
        probe
            .call("horse.set_level", &[Value::Number(-3.9)])
            .unwrap();
        probe
            .call("horse.set_level", &[Value::Number(12.9)])
            .unwrap();
        probe.call("horse.advance", &[]).unwrap();
        assert_eq!(probe.game.horse_levels, [30, 0, 12, 13]);
        probe.game.horse.level = 30;
        probe.call("horse.advance", &[]).unwrap();
        assert_eq!(probe.game.horse_levels.len(), 4, "30 advances no further");
        probe.game.horse.health = 0;
        assert_eq!(
            probe.call("horse.is_dead", &[]).unwrap(),
            [Value::Boolean(true)]
        );
        probe.call("horse.feed", &[]).unwrap();
        assert_eq!(probe.game.horse_feeds, 1, "a dead horse is not fed");
    }

    /// The summon names: `horse.summon` reads a boolean and a number, `horse.revive` revives
    /// only a dead horse with a level, `pc.revive_horse` always asks, `horse.ride` rides
    /// except on the OX map, `horse.unride` always asks, and `is_summon` and `is_mine`
    /// read the summoned horse against the quest's NPC.
    #[test]
    fn the_horse_summon_names_read_their_arguments_and_the_npc() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        probe.game.horse = HorseView {
            level: 21,
            health: 13,
            stamina: 250,
            max_health: 40,
            max_stamina: 140,
            riding: false,
            summoned: 0x0102_0304,
        };
        let word = Value::String(lua.create_string(b"20103").unwrap());
        probe
            .call("horse.summon", &[Value::Boolean(true), word])
            .unwrap();
        probe
            .call("horse.summon", &[Value::Number(1.0), Value::Nil])
            .unwrap();
        probe.call("horse.unsummon", &[]).unwrap();
        probe.call("horse.revive", &[]).unwrap();
        probe.game.horse.health = 0;
        probe.call("horse.revive", &[]).unwrap();
        probe.call("pc.revive_horse", &[]).unwrap();
        probe.call("horse.ride", &[]).unwrap();
        probe.call("horse.unride", &[]).unwrap();
        probe.game.map_index = 113;
        probe.call("horse.ride", &[]).unwrap();
        assert_eq!(
            probe.game.horse_calls,
            [
                ("summon", true, 20103),
                ("summon", false, 0),
                ("unsummon", false, 0),
                ("revive", false, 0),
                ("revive", false, 0),
                ("ride", false, 0),
                ("unride", false, 0),
            ]
        );
        assert_eq!(
            probe.call("horse.is_summon", &[]).unwrap(),
            [Value::Boolean(true)]
        );
        assert_eq!(
            probe.call("horse.is_mine", &[]).unwrap(),
            [Value::Boolean(false)]
        );
    }

    /// The name names: `horse.set_name` pushes nothing for a non-string, else the world's
    /// answer, and `horse.get_name` pushes the manager's name.
    #[test]
    fn the_horse_name_names_push_the_worlds_answers() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        assert!(probe
            .call("horse.set_name", &[Value::Number(7.0)])
            .unwrap()
            .is_empty());
        assert!(probe.game.horse_calls.is_empty(), "no string, no naming");
        let shadow = lua.create_string(b"Shadowfax").unwrap();
        assert_eq!(
            probe
                .call("horse.set_name", &[Value::String(shadow)])
                .unwrap(),
            [Value::Number(2.0)]
        );
        assert_eq!(probe.game.horse_names, b"Shadowfax");
        assert_eq!(
            probe.game.horse_calls,
            [("set_name", false, 0)],
            "the name is queued"
        );
        let pushed = probe.call("horse.get_name", &[]).unwrap();
        assert_eq!(pushed.len(), 1);
        let Value::String(name) = &pushed[0] else {
            panic!("a string, not {pushed:?}");
        };
        assert_eq!(name.as_bytes(), b"Shadowfax");
    }

    /// The mount names: `pc.mount` queues nothing without a number first, 60 seconds
    /// without a second number or below 0, and the vnum as a `DWORD`; `pc.mount_bonus`
    /// keeps the low byte of its apply type; `pc.unmount` queues the dismount.
    #[test]
    fn the_mount_names_read_their_arguments() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::default(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let word = Value::String(lua.create_string(b"horse").unwrap());
        let numeric = Value::String(lua.create_string(b"20205").unwrap());
        let calls: [(&str, Vec<Value>); 6] = [
            ("pc.mount", vec![word.clone(), Value::Number(5.0)]),
            ("pc.mount", vec![Value::Number(20_201.0)]),
            ("pc.mount", vec![numeric, Value::Number(-1.0)]),
            ("pc.mount", vec![Value::Number(-1.0), Value::Number(45.0)]),
            (
                "pc.mount_bonus",
                vec![Value::Number(264.0), Value::Number(10.0)],
            ),
            ("pc.unmount", vec![word]),
        ];
        for (name, args) in calls {
            assert!(probe.call(name, &args).unwrap().is_empty(), "{name}");
        }
        assert_eq!(
            probe.game.mount_calls,
            [
                ("mount", 20_201, 60, 0),
                ("mount", 20_205, 60, 0),
                ("mount", u32::MAX, 45, 0),
                ("mount_bonus", 8, 10, 0),
                ("unmount", 0, 0, 0),
            ]
        );
    }

    /// The tail names: the IDs answer the Fake's, the empty count and the trade and
    /// busy gates answer its flags, `give_exp2` queues its `rint` grant and a
    /// non-number runs nothing, the skill group keeps its low byte, the local warp
    /// keeps its `rint` triple (`rint` rounds half to even), `select` answers the
    /// old VID (0: the Fake starts vidless) and 0 without one, the killee answers
    /// its pair, and the plain drop queues its vnum and count (a text vnum reads 0).
    #[test]
    fn the_tail_names_read_their_arguments() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let mut probe = Probe {
            manager: &manager,
            game: Fake {
                player_id: 9,
                account_table_id: 4,
                empty_cells: 171,
                trading: true,
                busy: true,
                killee: (1500, 4000),
                vids: [(4321, 11)].into_iter().collect(),
                ..Fake::default()
            },
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let num = |value: f64| Value::Number(value);
        let one = |values: Vec<Value>| values;
        assert_eq!(
            one(probe.call("pc.get_player_id", &[]).unwrap()),
            [num(9.0)]
        );
        assert_eq!(
            one(probe.call("pc.get_account_id", &[]).unwrap()),
            [num(4.0)]
        );
        assert_eq!(
            one(probe.call("pc.get_empty_inventory_count", &[]).unwrap()),
            [num(171.0)]
        );
        assert_eq!(
            one(probe.call("pc.is_trade0", &[]).unwrap()),
            [Value::Boolean(true)]
        );
        assert_eq!(
            one(probe.call("pc.is_busy0", &[]).unwrap()),
            [Value::Boolean(true)]
        );
        let text = |bytes: &[u8]| Value::String(lua.create_string(bytes).unwrap());
        for (name, args) in [
            ("pc.give_exp2", vec![num(2500.6)]),
            ("pc.give_exp2", vec![text(b"nope")]),
            ("pc.set_skill_group", vec![num(513.0)]),
            ("pc.warp_local", vec![num(1.0), num(100.4), num(200.5)]),
            ("game.drop_item", vec![num(19.0), num(3.0)]),
            ("game.drop_item", vec![text(b"vnum?"), num(3.0)]),
        ] {
            assert!(probe.call(name, &args).unwrap().is_empty(), "{name}");
        }
        assert_eq!(probe.game.exp2, [2501]);
        assert_eq!(probe.game.skill_groups, [1]);
        assert_eq!(probe.game.local_warps, [(1, 100, 200)]);
        assert_eq!(probe.game.drops_plain, [(19, 3), (0, 3)]);
        assert_eq!(
            one(probe.call("pc.select", &[num(4321.0)]).unwrap()),
            [num(0.0)]
        );
        assert_eq!(
            one(probe.call("pc.select", &[num(999.0)]).unwrap()),
            [num(0.0)]
        );
        assert_eq!(
            one(probe.call("pc.get_killee_drop_pct", &[]).unwrap()),
            [num(1500.0), num(4000.0)]
        );
    }

    /// The attr67 names: the window's readers answer the Fake's vnum, `success`
    /// rolls the rare only on a won flag with the control at 0 or below, and
    /// `collect` moves the flags out on a move and chats with no room.
    #[test]
    fn the_attr67_names_read_the_window_and_the_flags() {
        let manager = manager().lock().unwrap();
        let mut probe = Probe {
            manager: &manager,
            game: Fake {
                attr67_vnum: 10,
                attr67_moved: true,
                ..Fake::default()
            },
            output: Output::default(),
            borrowed: BTreeMap::from([(
                PID,
                BTreeMap::from([
                    (b"attr67add_item.success".to_vec(), 1),
                    (b"attr67add_item.wait_time".to_vec(), 999),
                    (b"attr67add_item.add".to_vec(), 0),
                ]),
            )]),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let one = |values: Vec<Value>| values;
        assert_eq!(
            one(probe.call("attr67add.holding", &[]).unwrap()),
            [Value::Boolean(true)]
        );
        assert_eq!(
            one(probe.call("attr67add.item_vnum", &[]).unwrap()),
            [Value::Number(10.0)]
        );
        assert_eq!(
            one(probe.call("attr67add.window_type", &[]).unwrap()),
            [Value::Number(6.0)]
        );
        assert_eq!(
            one(probe.call("attr67add.window_cell", &[]).unwrap()),
            [Value::Number(0.0)]
        );
        assert_eq!(
            one(probe.call("attr67add.success", &[]).unwrap()),
            [Value::Boolean(true)]
        );
        assert_eq!(probe.game.attr67_rares, 1, "the first success rolls");
        assert_eq!(
            one(probe.call("attr67add.success", &[]).unwrap()),
            [Value::Boolean(true)]
        );
        assert_eq!(probe.game.attr67_rares, 1, "the control holds the second");
        probe.call("attr67add.collect", &[]).unwrap();
        assert_eq!(probe.game.attr67_collects, 1);
        assert!(
            probe.borrowed.get(&PID).is_some_and(BTreeMap::is_empty),
            "the move resets the three flags"
        );
    }

    /// The attr67 refusals: no window item answers false and pushes nothing, a lost
    /// flag answers false without a roll, and a stuck collect chats the line.
    #[test]
    fn the_attr67_names_refuse_without_a_window_or_a_win() {
        let manager = manager().lock().unwrap();
        let mut probe = Probe {
            manager: &manager,
            game: Fake {
                attr67_moved: false,
                ..Fake::default()
            },
            output: Output::default(),
            borrowed: BTreeMap::from([(
                PID,
                BTreeMap::from([(b"attr67add_item.success".to_vec(), 0)]),
            )]),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let one = |values: Vec<Value>| values;
        assert_eq!(
            one(probe.call("attr67add.holding", &[]).unwrap()),
            [Value::Boolean(false)]
        );
        assert_eq!(
            one(probe.call("attr67add.success", &[]).unwrap()),
            [Value::Boolean(false)]
        );
        assert_eq!(probe.game.attr67_rares, 0, "no window rolls nothing");
        assert!(
            probe.call("attr67add.collect", &[]).unwrap().is_empty(),
            "no window pushes nothing"
        );
        probe.game.attr67_vnum = 10;
        assert_eq!(
            one(probe.call("attr67add.success", &[]).unwrap()),
            [Value::Boolean(false)],
            "a lost flag answers false"
        );
        assert_eq!(probe.game.attr67_rares, 0, "a lost flag rolls nothing");
        probe.call("attr67add.collect", &[]).unwrap();
        assert_eq!(
            probe.game.chats,
            [(ChatKind::Info, b"Not enough inventory space.".to_vec())]
        );
    }

    /// The observer warp's errors: a non-number errors `bad argument #N`, an unknown
    /// guild (the Fake names none) errors `no such guild`, and a known pair warps —
    /// the Fake's `guild_known` is flipped through a second probe.
    #[test]
    fn the_observer_warp_names_its_errors() {
        let manager = manager().lock().unwrap();
        let mut probe = Probe {
            manager: &manager,
            game: Fake {
                guild_known: false,
                ..Fake::default()
            },
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let word = Value::String(manager.host.lua().create_string(b"seven").unwrap());
        let error = probe
            .call(
                "pc.warp_to_guild_war_observer_position",
                &[word, Value::Number(2.0)],
            )
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("bad argument #1") && error.contains("got string"),
            "{error}"
        );
        let error = probe
            .call(
                "pc.warp_to_guild_war_observer_position",
                &[Value::Number(7.0)],
            )
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("bad argument #2") && error.contains("got no value"),
            "{error}"
        );
        let error = probe
            .call(
                "pc.warp_to_guild_war_observer_position",
                &[Value::Number(7.0), Value::Number(9.0)],
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("no such guild with id 7 9"), "{error}");
        // The war with no map: known guilds, but the Fake is hacked, so no
        // position — `not under warp guild war between guild`.
        probe.game.guild_known = true;
        probe.game.hack = true;
        let error = probe
            .call(
                "pc.warp_to_guild_war_observer_position",
                &[Value::Number(7.0), Value::Number(9.0)],
            )
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("not under warp guild war between guild 7 9"),
            "{error}"
        );
        // Unhacked, the pair warps and pushes nothing.
        probe.game.hack = false;
        let pushed = probe
            .call(
                "pc.warp_to_guild_war_observer_position",
                &[Value::Number(7.0), Value::Number(9.0)],
            )
            .unwrap();
        assert!(pushed.is_empty(), "the warp pushes nothing");
        assert_eq!(probe.game.observes, [(7, 9), (7, 9), (7, 9)]);
    }

    /// `d.select`'s adoption (`G/questmanager.cpp:1252-1264`): with a character the
    /// select answers but the session keeps its own dungeon; with none (PID 0) the
    /// session adopts the living index, or 0 when the select missed.
    #[test]
    fn the_select_adopts_only_with_no_character() {
        let manager = manager().lock().unwrap();
        let mut probe = Probe {
            manager: &manager,
            game: Fake {
                dungeon: true,
                dungeon_index: 11,
                ..Fake::default()
            },
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let index = vec![Value::Number(2_100_000.0)];
        assert_eq!(
            probe.call("d.select", &index).unwrap(),
            [Value::Boolean(true)]
        );
        assert_eq!(probe.game.dungeon_index, 11, "a character keeps its own");
        probe.current = 0;
        assert_eq!(
            probe.call("d.select", &index).unwrap(),
            [Value::Boolean(true)]
        );
        assert_eq!(probe.game.dungeon_index, 2_100_000, "PID 0 adopts");
        probe.game.dungeon = false;
        assert_eq!(
            probe.call("d.select", &index).unwrap(),
            [Value::Boolean(false)]
        );
        assert_eq!(probe.game.dungeon_index, 0, "a miss clears");
    }

    /// The rank names: `pc.global_rank_add_point` queues nothing unless both arguments
    /// are numbers, and keeps the low byte of the counter (`(BYTE)`); a reward answers 1
    /// when the game pays and 0 when it does not; `pc.in_dungeon` is always false; the
    /// `npc.is_*` reads push `nil` with no NPC, and `is_pc`, `is_metin` and `is_boss`
    /// answer what the game holds of the NPC.
    #[test]
    fn the_rank_names_read_their_arguments() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::default(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        for args in [
            vec![],
            vec![Value::Number(1.0)],
            vec![
                Value::String(lua.create_string(b"two").unwrap()),
                Value::Number(1.0),
            ],
        ] {
            assert!(probe
                .call("pc.global_rank_add_point", &args)
                .unwrap()
                .is_empty());
        }
        probe
            .call(
                "pc.global_rank_add_point",
                &[Value::Number(258.0), Value::Number(3.0)],
            )
            .unwrap();
        assert_eq!(probe.game.rank_points, [(2, 3)]);
        assert_eq!(
            probe.call("pc.global_rank_get_reward", &[]).unwrap(),
            [Value::Number(0.0)]
        );
        probe.game.rank_reward = true;
        assert_eq!(
            probe.call("pc.global_rank_get_reward", &[]).unwrap(),
            [Value::Number(1.0)]
        );
        probe.game.rank_reward = false;
        assert_eq!(
            probe.call("pc.global_rank_get_reward", &[]).unwrap(),
            [Value::Number(0.0)]
        );
        assert_eq!(
            probe.call("pc.in_dungeon", &[]).unwrap(),
            [Value::Boolean(false)]
        );
        // The probe always reads the NPC at `VID`: a PC is no stone, a knight is no
        // boss, a stone is no PC, and a boss reads as one.
        probe.game.npc_pc = VID;
        probe.game.npc_kinds.insert(VID, (false, 3));
        assert_eq!(
            probe.call("npc.is_pc", &[]).unwrap(),
            [Value::Boolean(true)]
        );
        assert_eq!(
            probe.call("npc.is_metin", &[]).unwrap(),
            [Value::Boolean(false)]
        );
        assert_eq!(
            probe.call("npc.is_boss", &[]).unwrap(),
            [Value::Boolean(false)]
        );
        probe.game.npc_pc = 0;
        probe.game.npc_kinds.insert(VID, (true, 0));
        assert_eq!(
            probe.call("npc.is_pc", &[]).unwrap(),
            [Value::Boolean(false)]
        );
        assert_eq!(
            probe.call("npc.is_metin", &[]).unwrap(),
            [Value::Boolean(true)]
        );
        probe.game.npc_kinds.insert(VID, (false, 4));
        assert_eq!(
            probe.call("npc.is_boss", &[]).unwrap(),
            [Value::Boolean(true)]
        );
        probe.game.npc_kinds.remove(&VID);
        assert!(probe.call("npc.is_boss", &[]).unwrap().is_empty());
        assert!(probe.call("npc.is_metin", &[]).unwrap().is_empty());
    }

    /// The character-reader names: gold under its four spellings, empire, hp/sp and their
    /// maxima, exp and next exp, race, name, VID, leadership, the position pairs, the worn
    /// vnums (0 when empty, nil from `get_wear` for a bad cell, nothing pushed for a
    /// non-number cell), the shown alignment, and the NPC's empire.
    #[test]
    fn the_reader_names_answer_what_the_game_holds() {
        let manager = manager().lock().unwrap();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake {
                gold: 200_001,
                empire: 2,
                hp: 1234,
                max_hp: 2345,
                sp: 345,
                max_sp: 456,
                exp: 789_012,
                next_exp: 1_000_000,
                race: 5,
                char_name: b"Zulu".to_vec(),
                char_vid: 4321,
                leadership: 7,
                position: (4701, 9500, 101, 300),
                worn: [(0, 11209), (4, 19)].into_iter().collect(),
                empires: [(VID, 1)].into_iter().collect(),
                alignment: -12_345,
                ..Fake::default()
            },
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        for name in ["pc.getmoney", "pc.get_money", "pc.getgold", "pc.get_gold"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(200_001.0), "{name}");
        }
        for name in ["pc.getempire", "pc.get_empire"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(2.0), "{name}");
        }
        for name in ["pc.gethp", "pc.get_hp"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(1234.0), "{name}");
        }
        for name in ["pc.getmaxhp", "pc.get_max_hp"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(2345.0), "{name}");
        }
        for name in ["pc.getsp", "pc.get_sp"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(345.0), "{name}");
        }
        for name in ["pc.getmaxsp", "pc.get_max_sp"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(456.0), "{name}");
        }
        assert_eq!(probe.call("pc.get_exp", &[]).unwrap(), number(789_012.0));
        assert_eq!(
            probe.call("pc.get_next_exp", &[]).unwrap(),
            number(1_000_000.0)
        );
        assert_eq!(probe.call("pc.get_race", &[]).unwrap(), number(5.0));
        assert_eq!(probe.text("pc.get_name", &[]), b"Zulu");
        assert_eq!(probe.text("pc.getname", &[]), b"Zulu");
        assert_eq!(probe.call("pc.get_vid", &[]).unwrap(), number(4321.0));
        for name in ["pc.getleadership", "pc.get_leadership"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(7.0), "{name}");
        }
        the_positions_and_worn(&manager, &mut probe, &number);
        the_npc_empire(&mut probe, &number);
        the_guild_triads(&mut probe, &number);
        the_mount_and_party(&mut probe);
    }

    /// The NPC's empire among the reader names: the NPC's empire, 0 with no NPC.
    fn the_npc_empire(probe: &mut Probe<'_>, number: &dyn Fn(f64) -> Vec<Value>) {
        assert_eq!(probe.call("npc.get_empire", &[]).unwrap(), number(1.0));
        probe.game.empires.clear();
        assert_eq!(
            probe.call("npc.get_empire", &[]).unwrap(),
            number(0.0),
            "no NPC is 0"
        );
    }

    /// The positions, the worn vnums, and the shown alignment among the reader names.
    fn the_positions_and_worn(
        manager: &std::sync::MutexGuard<'_, Manager>,
        probe: &mut Probe<'_>,
        number: &dyn Fn(f64) -> Vec<Value>,
    ) {
        for name in ["pc.get_x", "pc.getx"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(4701.0), "{name}");
        }
        for name in ["pc.get_y", "pc.gety"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(9500.0), "{name}");
        }
        assert_eq!(probe.call("pc.get_local_x", &[]).unwrap(), number(101.0));
        assert_eq!(probe.call("pc.get_local_y", &[]).unwrap(), number(300.0));
        for name in ["pc.getweapon", "pc.get_weapon"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(19.0), "{name}");
        }
        for name in ["pc.getarmor", "pc.get_armor"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(11209.0), "{name}");
        }
        assert_eq!(
            probe.call("pc.get_wear", &[Value::Number(4.0)]).unwrap(),
            number(19.0)
        );
        assert_eq!(
            probe.call("pc.get_wear", &[Value::Number(5.0)]).unwrap(),
            [Value::Nil],
            "an empty cell is nil"
        );
        assert_eq!(
            probe.call("pc.get_wear", &[Value::Number(64.0)]).unwrap(),
            [Value::Nil],
            "a cell past the bound is nil"
        );
        let table = Value::Table(manager.host.lua().create_table().unwrap());
        assert!(
            probe.call("pc.get_wear", &[table]).unwrap().is_empty(),
            "a non-number pushes nothing"
        );
        assert!(
            probe.call("pc.get_wear", &[]).unwrap().is_empty(),
            "a missing cell pushes nothing"
        );
        assert_eq!(
            probe.call("pc.get_alignment", &[]).unwrap(),
            number(-1234.0),
            "the shown /10 truncates toward zero"
        );
    }

    /// The guild triad among the reader names: `pc.has_guild`/`pc.hasguild` answer whether
    /// a guild id is held, `pc.isguildmaster`/`pc.is_guild_master` answer the master flag,
    /// and `pc.get_guild`/`pc.getguild` answer the id, 0 with none.
    fn the_guild_triads(probe: &mut Probe<'_>, number: &dyn Fn(f64) -> Vec<Value>) {
        for name in ["pc.has_guild", "pc.hasguild"] {
            assert_eq!(
                probe.call(name, &[]).unwrap(),
                [Value::Boolean(false)],
                "{name} with no guild"
            );
        }
        for name in ["pc.isguildmaster", "pc.is_guild_master"] {
            assert_eq!(
                probe.call(name, &[]).unwrap(),
                [Value::Boolean(false)],
                "{name} with no guild"
            );
        }
        for name in ["pc.get_guild", "pc.getguild"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(0.0), "{name}");
        }
        probe.game.guild_id = 42;
        probe.game.is_guild_master = true;
        assert_eq!(
            probe.call("pc.has_guild", &[]).unwrap(),
            [Value::Boolean(true)],
            "a guild id is a guild"
        );
        assert_eq!(
            probe.call("pc.is_guild_master", &[]).unwrap(),
            [Value::Boolean(true)],
            "the master flag"
        );
        assert_eq!(probe.call("pc.get_guild", &[]).unwrap(), number(42.0));
    }

    /// The mount and the party among the reader names: `pc.is_mount` answers the
    /// mount flag, `party.is_party` the party flag, and `party.is_leader` the
    /// leader flag.
    fn the_mount_and_party(probe: &mut Probe<'_>) {
        assert_eq!(
            probe.call("pc.is_mount", &[]).unwrap(),
            [Value::Boolean(false)]
        );
        assert_eq!(
            probe.call("party.is_party", &[]).unwrap(),
            [Value::Boolean(false)]
        );
        assert_eq!(
            probe.call("party.is_leader", &[]).unwrap(),
            [Value::Boolean(false)]
        );
        probe.game.mounted = true;
        probe.game.party = true;
        probe.game.party_leader = true;
        assert_eq!(
            probe.call("pc.is_mount", &[]).unwrap(),
            [Value::Boolean(true)],
            "mounted"
        );
        assert_eq!(
            probe.call("party.is_party", &[]).unwrap(),
            [Value::Boolean(true)],
            "in a party"
        );
        assert_eq!(
            probe.call("party.is_leader", &[]).unwrap(),
            [Value::Boolean(true)],
            "leading it"
        );
        probe.game.channel = 3;
        assert_eq!(
            probe.call("pc.get_channel_id", &[]).unwrap(),
            [Value::Number(3.0)],
            "the Channel"
        );
        the_party_flags(probe);
        the_dungeon_tail_names(probe);
    }

    /// The dungeon tail: `d.new_jump` stages its three numbers and pushes
    /// nothing, quiet without a number; `d.clear_regen` and `d.kill_all` run
    /// and push nothing; the two damage setters take the flag then the VID as
    /// `(DWORD)` and push nothing, quiet without a number.
    fn the_dungeon_tail_names(probe: &mut Probe<'_>) {
        let triple = [
            Value::Number(210.0),
            Value::Number(7824.0),
            Value::Number(15021.0),
        ];
        assert!(probe.call("d.new_jump", &triple).unwrap().is_empty());
        assert_eq!(probe.game.dungeon_singles, [(210, 7824, 15021)]);
        assert!(probe
            .call(
                "d.new_jump",
                &[Value::Number(210.0), Value::Nil, Value::Nil]
            )
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.dungeon_singles.len(), 1, "quiet without numbers");
        assert!(probe.call("d.clear_regen", &[]).unwrap().is_empty());
        assert_eq!(probe.game.dungeon_clears, 1);
        assert!(probe.call("d.kill_all", &[]).unwrap().is_empty());
        assert_eq!(probe.game.dungeon_kills, 1);
        for name in ["d.set_damage_from_race", "d.set_damage_from_affect"] {
            assert!(probe
                .call(name, &[Value::Number(3.0), Value::Number(777.0)])
                .unwrap()
                .is_empty());
            assert!(probe
                .call(name, &[Value::Number(3.0), Value::Nil])
                .unwrap()
                .is_empty());
        }
        assert_eq!(probe.game.dungeon_damage_race, [(3, 777)]);
        assert_eq!(probe.game.dungeon_damage_affect, [(3, 777)]);
    }

    /// The party flags: `party.get_leader_pid` answers the leader PID,
    /// `party.getf` answers the flag or 0, and `party.setf` sets a string name
    /// with a number value and pushes nothing.
    fn the_party_flags(probe: &mut Probe<'_>) {
        let flag = Value::String(
            probe
                .manager
                .host
                .lua()
                .create_string(b"frt_dungeon_index")
                .unwrap(),
        );
        assert_eq!(
            probe.call("party.get_leader_pid", &[]).unwrap(),
            [Value::Number(0.0)]
        );
        assert_eq!(
            probe.call("party.getf", &[flag.clone()]).unwrap(),
            [Value::Number(0.0)],
            "no such flag is 0"
        );
        assert_eq!(
            probe.call("party.getf", &[Value::Number(1.0)]).unwrap(),
            [Value::Number(0.0)],
            "a non-string name is 0"
        );
        probe.game.leader_pid = 4321;
        assert_eq!(
            probe.call("party.get_leader_pid", &[]).unwrap(),
            [Value::Number(4321.0)]
        );
        let value = Value::Number(210.0);
        assert!(probe
            .call("party.setf", &[flag.clone(), value])
            .unwrap()
            .is_empty());
        assert_eq!(
            probe.call("party.getf", &[flag]).unwrap(),
            [Value::Number(210.0)]
        );
        assert!(probe
            .call("party.setf", &[Value::Number(1.0), Value::Number(2.0)])
            .unwrap()
            .is_empty());
        let other = Value::String(probe.manager.host.lua().create_string(b"other").unwrap());
        assert!(probe
            .call("party.setf", &[other, Value::Nil])
            .unwrap()
            .is_empty());
        for name in ["d.select", "d.find"] {
            assert_eq!(
                probe.call(name, &[Value::Number(0.0)]).unwrap(),
                [Value::Boolean(false)],
                "{name} with no dungeon"
            );
            assert_eq!(
                probe.call(name, &[Value::Nil]).unwrap(),
                [Value::Boolean(false)],
                "{name} with no number"
            );
        }
        probe.game.dungeon = true;
        assert_eq!(
            probe.call("d.find", &[Value::Number(216.0)]).unwrap(),
            [Value::Boolean(true)],
            "a living dungeon"
        );
        assert_eq!(
            probe.call("d.select", &[Value::Number(-1.0)]).unwrap(),
            [Value::Boolean(true)],
            "(DWORD) of -1"
        );
        the_member_pids_and_the_block(probe);
        the_ds_names(probe);
    }

    /// The `ds.*` names: `is_qualified` answers 1 or 0 as a number, the grant sets
    /// it, the refine open runs only with the qualification, and the attr-change
    /// open always runs. The three writers push nothing.
    fn the_ds_names(probe: &mut Probe<'_>) {
        assert_eq!(
            probe.call("ds.is_qualified", &[]).unwrap(),
            [Value::Number(0.0)]
        );
        assert!(probe.call("ds.open_refine_window", &[]).unwrap().is_empty());
        assert_eq!(probe.game.ds_refines, 0, "unqualified opens nothing");
        assert!(probe.call("ds.give_qualification", &[]).unwrap().is_empty());
        assert_eq!(probe.game.ds_gives, 1);
        assert_eq!(
            probe.call("ds.is_qualified", &[]).unwrap(),
            [Value::Number(1.0)]
        );
        assert!(probe.call("ds.open_refine_window", &[]).unwrap().is_empty());
        assert_eq!(probe.game.ds_refines, 1);
        assert!(probe
            .call("ds.open_attr_change_window", &[])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.ds_attrs, 1);
    }

    /// `party.get_member_pids` pushes the PIDs the game names, none with no party,
    /// and `q.begin_other_pc_block`/`q.end_other_pc_block` push nothing: the begin
    /// grows the stack even for an offline PID, the end restores the pushed PID,
    /// and a flag written in the block lands on the blocked PC's map.
    fn the_member_pids_and_the_block(probe: &mut Probe<'_>) {
        assert!(probe.call("party.get_member_pids", &[]).unwrap().is_empty());
        probe.game.member_pids = vec![11, 7, 23];
        assert_eq!(
            probe.call("party.get_member_pids", &[]).unwrap(),
            [Value::Number(11.0), Value::Number(7.0), Value::Number(23.0)]
        );
        let flag = Value::String(probe.manager.host.lua().create_string(b"flag").unwrap());
        let quest = Value::String(probe.manager.host.lua().create_string(b"q").unwrap());
        let one = Value::Number(1.0);
        assert!(probe
            .call("pc.setf", &[quest.clone(), flag.clone(), one.clone()])
            .unwrap()
            .is_empty());
        // Fake.blocked answers None: an offline PID grows the stack but the flag
        // still lands on the root's map.
        assert!(probe
            .call("q.begin_other_pc_block", &[Value::Number(23.0)])
            .unwrap()
            .is_empty());
        assert!(probe
            .call(
                "pc.setf",
                &[quest.clone(), flag.clone(), Value::Number(2.0)]
            )
            .unwrap()
            .is_empty());
        assert_eq!(
            probe
                .call("pc.getf", &[quest.clone(), flag.clone()])
                .unwrap(),
            [Value::Number(2.0)]
        );
        assert!(probe.call("q.end_other_pc_block", &[]).unwrap().is_empty());
        assert_eq!(probe.borrowed.len(), 1, "no blocked map was made");
        assert!(probe.call("q.end_other_pc_block", &[]).unwrap().is_empty());
        assert_eq!(
            probe
                .call("pc.getf", &[quest.clone(), flag.clone()])
                .unwrap(),
            [Value::Number(2.0)],
            "the root kept the write"
        );
        // An online PID switches: the write lands on the blocked map, and the end
        // reads the root's again.
        probe.game.online = vec![23];
        assert!(probe
            .call("q.begin_other_pc_block", &[Value::Number(23.0)])
            .unwrap()
            .is_empty());
        assert!(probe
            .call(
                "pc.setf",
                &[quest.clone(), flag.clone(), Value::Number(3.0)]
            )
            .unwrap()
            .is_empty());
        assert_eq!(
            probe
                .call("pc.getf", &[quest.clone(), flag.clone()])
                .unwrap(),
            [Value::Number(3.0)]
        );
        assert_eq!(probe.borrowed[&23].get(b"q.flag".as_slice()), Some(&3));
        assert!(probe.call("q.end_other_pc_block", &[]).unwrap().is_empty());
        assert_eq!(
            probe.call("pc.getf", &[quest, flag]).unwrap(),
            [Value::Number(2.0)],
            "the end reads the root's again"
        );
    }

    /// The alignment and no-delay book names: `pc.get_real_alignment` answers the stored value
    /// over ten, `pc.changealignment`/`pc.change_alignment` pass `(int)(number * 10)` (a
    /// non-number is zero), `pc.is_skill_book_no_delay` answers the affect, and
    /// `pc.remove_skill_book_no_delay` removes it.
    #[test]
    fn the_alignment_and_no_delay_book_names_follow_legacy() {
        let manager = manager().lock().unwrap();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        probe.game.alignment = -190_000;
        assert_eq!(
            probe.call("pc.get_real_alignment", &[]).unwrap(),
            number(-19_000.0)
        );
        probe.game.alignment = -5;
        assert_eq!(
            probe.call("pc.get_real_alignment", &[]).unwrap(),
            number(0.0),
            "the division truncates toward zero, as C's int / 10 does"
        );
        for name in ["pc.changealignment", "pc.change_alignment"] {
            let change = |probe: &mut Probe, args: &[Value]| probe.call(name, args).unwrap();
            assert!(change(&mut probe, &number(12.34)).is_empty());
            assert!(change(&mut probe, &number(-3.9)).is_empty());
            assert!(change(&mut probe, &[Value::Nil]).is_empty());
        }
        assert_eq!(
            probe.game.alignment_changes,
            [123, -39, 0, 123, -39, 0],
            "the amount is (int)(lua_tonumber * 10), and a non-number is zero"
        );
        assert_eq!(
            probe.call("pc.is_skill_book_no_delay", &[]).unwrap(),
            [Value::Boolean(false)]
        );
        probe.game.skill_book_no_delay = true;
        assert_eq!(
            probe.call("pc.is_skill_book_no_delay", &[]).unwrap(),
            [Value::Boolean(true)]
        );
        assert!(probe
            .call("pc.remove_skill_book_no_delay", &[])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.skill_book_removals, 1);
        assert!(!probe.game.skill_book_no_delay);
    }
    /// `pc_change_money` (`G/questlua_pc.cpp:1102-1121`) under its four names: `(long long)`
    /// of the last argument, so a fraction drops toward zero, a missing or non-number
    /// argument moves nothing, and nothing is pushed.
    #[test]
    fn the_money_change_passes_long_long_of_the_last_argument() {
        let manager = manager().lock().unwrap();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        for name in [
            "pc.changegold",
            "pc.changemoney",
            "pc.change_gold",
            "pc.change_money",
        ] {
            let change = |probe: &mut Probe, args: &[Value]| probe.call(name, args).unwrap();
            assert!(change(&mut probe, &number(12.9)).is_empty());
            assert!(change(&mut probe, &number(-3_000_000.9)).is_empty());
            assert!(change(&mut probe, &[]).is_empty());
            assert!(change(&mut probe, &[Value::Nil]).is_empty());
            assert!(change(&mut probe, &[Value::Number(1.0), Value::Number(2.0)]).is_empty());
        }
        assert_eq!(
            probe.game.gold_changes,
            [
                12, -3_000_000, 0, 0, 2, 12, -3_000_000, 0, 0, 2, 12, -3_000_000, 0, 0, 2, 12,
                -3_000_000, 0, 0, 2
            ],
            "the last argument, truncated toward zero"
        );
    }
    /// `pc_hide_costume` (`G/questlua_pc.cpp:3553-3600`): part 1-5 with a boolean hides or
    /// shows it and writes the `costume_option.hide_*` flag (a show deletes it). A
    /// non-number part, a non-boolean hide, or a part outside 1-5 changes nothing. Nothing
    /// is pushed.
    #[test]
    fn the_costume_hide_names_its_part_and_writes_its_flag() {
        let manager = manager().lock().unwrap();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let hide = |probe: &mut Probe, args: &[Value]| probe.call("pc.hide_costume", args).unwrap();
        let yes = Value::Boolean(true);
        let no = Value::Boolean(false);
        for (part, flag) in [
            (1.0, b"costume_option.hide_body".as_slice()),
            (2.0, b"costume_option.hide_hair".as_slice()),
            (3.0, b"costume_option.hide_sash".as_slice()),
            (4.0, b"costume_option.hide_aura".as_slice()),
            (5.0, b"costume_option.hide_weapon".as_slice()),
        ] {
            assert!(hide(&mut probe, &[Value::Number(part), yes.clone()]).is_empty());
            assert_eq!(probe.borrowed[&PID].get(flag), Some(&1), "hide writes");
            assert!(hide(&mut probe, &[Value::Number(part), no.clone()]).is_empty());
            assert_eq!(probe.borrowed[&PID].get(flag), None, "show deletes");
        }
        assert_eq!(
            probe.game.hides,
            [
                (1, true),
                (1, false),
                (2, true),
                (2, false),
                (3, true),
                (3, false),
                (4, true),
                (4, false),
                (5, true),
                (5, false)
            ]
        );
        let table = Value::Table(manager.host.lua().create_table().unwrap());
        assert!(hide(&mut probe, &[table.clone(), yes.clone()]).is_empty());
        assert!(hide(&mut probe, &[Value::Number(1.0), Value::Number(1.0)]).is_empty());
        assert!(hide(&mut probe, &[Value::Number(0.0), yes.clone()]).is_empty());
        assert!(hide(&mut probe, &[Value::Number(6.0), yes.clone()]).is_empty());
        assert!(hide(&mut probe, &[Value::Number(1.0)]).is_empty());
        assert!(hide(&mut probe, &[]).is_empty());
        assert_eq!(probe.game.hides.len(), 10, "no bad call hides");
        assert!(probe.borrowed[&PID].is_empty());
    }

    /// `pc_open_sash` takes only a boolean (combination or absorption) and
    /// `pc_open_changelook` reads no argument; both push nothing, and anything
    /// else logs and changes nothing.
    #[test]
    fn the_window_opens_take_their_arguments_and_push_nothing() {
        let manager = manager().lock().unwrap();
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let yes = Value::Boolean(true);
        let no = Value::Boolean(false);
        assert!(probe.call("pc.open_sash", &[yes]).unwrap().is_empty());
        assert!(probe.call("pc.open_sash", &[no]).unwrap().is_empty());
        assert_eq!(probe.game.sash_opens, [true, false]);
        assert!(probe.call("pc.open_changelook", &[]).unwrap().is_empty());
        assert!(probe
            .call("pc.open_changelook", &[Value::Number(1.0)])
            .unwrap()
            .is_empty());
        assert_eq!(probe.game.changelooks, 2, "no argument is read");
        let table = Value::Table(manager.host.lua().create_table().unwrap());
        assert!(probe
            .call("pc.open_sash", &[Value::Number(1.0)])
            .unwrap()
            .is_empty());
        assert!(probe.call("pc.open_sash", &[table]).unwrap().is_empty());
        assert!(probe.call("pc.open_sash", &[]).unwrap().is_empty());
        assert_eq!(probe.game.sash_opens.len(), 2, "no bad call opens");
    }

    /// `pc_send_block_mode` (`G/questlua_pc.cpp:2190-2197`): the number becomes
    /// the character's own block mode, dworded and low-byte only; a non-number
    #[test]
    fn the_block_mode_number_becomes_the_characters_own_byte() {
        let manager = manager().lock().unwrap();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let blocked =
            |probe: &mut Probe, args: &[Value]| probe.call("pc.send_block_mode", args).unwrap();
        assert!(blocked(&mut probe, &number(8.0)).is_empty());
        assert!(
            blocked(&mut probe, &number(264.9)).is_empty(),
            "dworded, then low byte only"
        );
        assert!(blocked(&mut probe, &[]).is_empty(), "a non-number is zero");
        assert_eq!(probe.game.blocked, [8, 8, 0]);
    }

    /// The skin index rounds ties to even and falls back to the normal skin; a value that
    /// is no number keeps the skin.
    #[test]
    fn the_skin_index_rounds_and_recovers() {
        let manager = manager().lock().unwrap();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        for (skin, want) in [
            (4.5, 4),
            (5.5, 1),
            (2.5, 2),
            (-0.4, 0),
            (6.0, 1),
            (f64::NAN, 1),
        ] {
            probe.call("set_skin", &number(skin)).unwrap();
            assert_eq!(probe.output.skin, want, "{skin}");
            // A value that is no number keeps the skin.
            probe.call("setskin", &[Value::Boolean(true)]).unwrap();
            assert_eq!(probe.output.skin, want, "{skin}");
        }
    }

    /// A level up moves the sash quest to `information` at 40 and the Yohara quest to
    /// `information_new` at 120; each change runs the new state's `enter` and `letter`
    /// scripts (`G/questpc.cpp:352-371`), and with no Theowahdan or statue on the map the
    /// letters only send their buttons and windows.
    #[test]
    fn a_level_up_runs_the_no_target_scripts_and_misses_an_unknown_character() {
        let mut manager = manager().lock().unwrap();
        let sash = manager.host().quest_index(b"sash_mission").expect("sash");
        let yohara = manager
            .host()
            .quest_index(b"yohara_system")
            .expect("yohara");
        let letter = |quest: u32, title: &str| {
            let script = format!("[QUESTBUTTON idx;{quest}|name;{title}][DONE]");
            let info = QuestInfo {
                index: u16::try_from(quest).expect("a small number"),
                category: 0,
                begin: None,
                title: Some(title.as_bytes().to_vec()),
                clock_name: None,
                clock_value: None,
                counter_name: None,
                counter_value: None,
            };
            ((QUEST_SKIN_NOWINDOW, script), info)
        };
        manager.disconnect(PID);
        let mut stranger = Fake::default();
        assert!(!manager.levelup(PID, &mut stranger));
        assert!(stranger.scripts.is_empty());
        let mut game = Fake::at_ox_manager();
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        game.scripts.clear();
        manager.disconnect(PID);
        manager.pcs.entry(PID).or_default();
        game.level = 40;
        assert!(manager.levelup(PID, &mut game));
        assert_eq!(
            manager.quest_flag(PID, b"sash_mission.__status"),
            Some(695_801_987),
            "the sash quest left `start` for CRC32(`information`)"
        );
        assert_eq!(
            manager.quest_flag(PID, b"yohara_system.__status"),
            Some(0),
            "the yohara quest stayed in `start` below 120"
        );
        let (script, info) = letter(sash, "A special cloth");
        assert_eq!(game.take(), [script]);
        assert_eq!(std::mem::take(&mut game.infos), [info]);
        assert!(game.created.is_empty(), "no Theowahdan on the map");
        manager.disconnect(PID);
        manager.pcs.entry(PID).or_default();
        game.level = 120;
        assert!(manager.levelup(PID, &mut game));
        assert_eq!(
            manager.quest_flag(PID, b"yohara_system.__status"),
            Some(1_998_515_313),
            "at 120 the yohara quest moves to CRC32(`information_new`)"
        );
        let (sash_script, sash_info) = letter(sash, "A special cloth");
        let (yohara_script, yohara_info) = letter(yohara, "Statuia Razboinicului");
        assert_eq!(game.take(), [sash_script, yohara_script]);
        assert_eq!(game.infos, [sash_info, yohara_info]);
        assert!(game.created.is_empty() && game.deleted.is_empty());
        manager.disconnect(PID);
    }

    /// The sash quest's state change at 40 (`G/questpc.cpp:352-371`): its `enter` script
    /// clears two flags with `pc.setqf` (`SetFlag` deletes a zero), and its `letter` script
    /// finds Theowahdan on the map (`find_npc_by_vnum`) and marks him with `target.vid` before
    /// it sends its button; the window follows the button. A login at the new state runs only
    /// the letter pass, whose target replaces the one of the same name.
    #[test]
    fn a_state_change_runs_the_new_states_enter_and_letter() {
        const THEOWAHDAN: u32 = 0x8000_0007;
        let mut manager = manager().lock().unwrap();
        let sash = manager.host().quest_index(b"sash_mission").expect("sash");
        manager.disconnect(PID);
        let stored = [
            (b"sash_mission.time_to_wait".to_vec(), 5),
            (b"sash_mission.collect_count".to_vec(), 3),
            (b"sash_mission.other".to_vec(), 9),
        ];
        manager.load_flags(PID, stored);
        let mut game = Fake {
            level: 40,
            map_index: 1,
            npcs: [(60003, THEOWAHDAN)].into_iter().collect(),
            ..Fake::default()
        };
        assert!(manager.levelup(PID, &mut game));
        assert_eq!(manager.quest_flag(PID, b"sash_mission.time_to_wait"), None);
        assert_eq!(manager.quest_flag(PID, b"sash_mission.collect_count"), None);
        assert_eq!(manager.quest_flag(PID, b"sash_mission.other"), Some(9));
        let target = Target {
            quest: sash,
            name: b"theowahdan".to_vec(),
            vid: THEOWAHDAN,
            desc: Some(b"Theowahdan".to_vec()),
            send: true,
        };
        assert_eq!(game.created, [target.clone()]);
        let button = format!("[QUESTBUTTON idx;{sash}|name;A special cloth][DONE]");
        assert_eq!(game.take(), [(QUEST_SKIN_NOWINDOW, button.clone())]);
        assert_eq!(game.infos.len(), 1);

        // The login finds the quest at `information`: no change, only the letter.
        game.created.clear();
        game.infos.clear();
        let flags = manager.flags_of(PID);
        manager.disconnect(PID);
        manager.load_flags(PID, flags);
        // The sash's own `login` script is `start`'s; other quests' run.
        manager.login(PID, &mut game);
        assert_eq!(game.created, [target.clone()]);
        assert_eq!(game.targets, [target]);
        let scripts: Vec<_> = game.take().into_iter().map(|(_, text)| text).collect();
        assert!(scripts.contains(&button), "{scripts:?}");
        manager.disconnect(PID);
    }

    /// A manager over a locale of the test's own: empty libraries and the one quest `source`.
    fn synthetic(name: &str, source: &str) -> Manager {
        let root =
            std::env::temp_dir().join(format!("quest-manager-{}-{name}", std::process::id()));
        let files = [
            ("settings.lua", ""),
            ("translate.lua", ""),
            ("quest/questlib.lua", ""),
            ("quest/locale.lua", ""),
            ("quest/quest_list", ""),
            ("quest/test/test.quest", source),
        ];
        for (path, text) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let host = Host::load(&root);
        fs::remove_dir_all(&root).unwrap();
        Manager::new(host.unwrap(), b"", b"")
    }

    /// `EndRunning`'s state change (`G/questpc.cpp:352-371`), nested as legacy nests it: the old
    /// state's `leave`, then the new state's `enter`, then its `letter` only while the status
    /// flag still names the new state, so an `enter` that moves the quest on again skips its
    /// own letter, and a script that names its own state runs no change script. Changes that
    /// keep moving the quest stop 16 deep (`STATE_CHANGE_DEPTH`, a Divergence: legacy recursed
    /// until its stack ran out).
    #[test]
    fn a_state_change_leaves_enters_and_letters_in_legacys_nesting() {
        const FLIP: &str = r#"
quest flip begin
    state start begin
        when login begin
            set_state("c")
        end
    end
    state c begin
        when leave begin chat("leave c") end
        when enter begin
            chat("enter c")
            set_state("d")
        end
        when letter begin chat("letter c") end
    end
    state d begin
        when leave begin chat("leave d") end
        when enter begin chat("enter d") end
        when letter begin chat("letter d") end
        when kill begin
            chat("kill d")
            set_state("d")
        end
        when levelup begin set_state("a") end
    end
    state a begin
        when leave begin chat("leave a") end
        when enter begin
            chat("enter a")
            set_state("b")
        end
    end
    state b begin
        when leave begin chat("leave b") end
        when enter begin
            chat("enter b")
            set_state("a")
        end
    end
end
"#;
        let mut manager = synthetic("flip", FLIP);
        let states = manager.host().quests()[0].states.clone();
        let chats = |game: &mut Fake| -> Vec<String> {
            game.chats
                .drain(..)
                .map(|(_, text)| String::from_utf8_lossy(&text).into_owned())
                .collect()
        };
        manager.load_flags(PID, Vec::new());
        let mut game = Fake::default();
        assert!(manager.login(PID, &mut game));
        // `start` has no `leave`; `enter c` moves on to `d`, so `c` gets no letter; the login's
        // letter pass then runs `d`'s again.
        let want = ["enter c", "leave c", "enter d", "letter d", "letter d"];
        assert_eq!(chats(&mut game), want);
        assert_eq!(
            manager.quest_flag(PID, b"flip.__status"),
            Some(states[&b"d"[..]])
        );

        // A script that names its own state changes nothing, so no change script runs.
        assert!(manager.kill(PID, 101, None, &mut game));
        assert_eq!(chats(&mut game), ["kill d"]);
        assert_eq!(
            manager.quest_flag(PID, b"flip.__status"),
            Some(states[&b"d"[..]])
        );

        assert!(manager.levelup(PID, &mut game));
        let mut want = vec!["leave d".to_owned(), "enter a".to_owned()];
        for depth in 1..STATE_CHANGE_DEPTH {
            let (from, to) = if depth % 2 == 1 {
                ("a", "b")
            } else {
                ("b", "a")
            };
            want.push(format!("leave {from}"));
            want.push(format!("enter {to}"));
        }
        assert_eq!(chats(&mut game), want);
        assert_eq!(
            manager.quest_flag(PID, b"flip.__status"),
            Some(states[&b"a"[..]]),
            "the change the guard stopped still lands"
        );
        assert!(!manager.is_running(PID));
    }

    /// `pc_set_quest_flag` (`G/questlua_pc.cpp:1215-1229`): text or a number names a flag of
    /// the quest that runs and a number is its value, rounded half to even and cast to `int`
    /// (`int(rint(...))`); 0 deletes the flag, and a missing name or value changes nothing.
    #[test]
    fn setqf_rounds_to_even_and_a_zero_deletes() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let table = Value::Table(lua.create_table().unwrap());
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let mut set = |name: Value, value: Value| {
            let answer = probe.call("pc.setqf", &[name, value]).unwrap();
            assert!(answer.is_empty());
            probe.borrowed[&PID].get(b"q.a".as_slice()).copied()
        };
        assert_eq!(set(string(b"a"), Value::Number(2.5)), Some(2));
        assert_eq!(set(string(b"a"), Value::Number(3.5)), Some(4));
        assert_eq!(set(string(b"a"), Value::Number(-2.5)), Some(-2));
        assert_eq!(set(string(b"a"), string(b"7")), Some(7));
        assert_eq!(
            set(string(b"a\0b"), Value::Number(8.0)),
            Some(8),
            "up to the zero"
        );
        assert_eq!(set(string(b"a"), Value::Number(f64::NAN)), Some(i32::MIN));
        assert_eq!(set(string(b"a"), Value::Number(3e9)), Some(i32::MIN));
        assert_eq!(set(string(b"a"), table.clone()), Some(i32::MIN), "no value");
        assert_eq!(
            set(table.clone(), Value::Number(1.0)),
            Some(i32::MIN),
            "no name"
        );
        assert_eq!(
            set(string(b"a"), Value::Number(0.4)),
            None,
            "a zero deletes"
        );
        assert_eq!(set(Value::Number(1.0), Value::Number(6.0)), None);
        assert_eq!(
            probe.borrowed[&PID].get(b"q.1".as_slice()),
            Some(&6),
            "a number names"
        );
        assert_eq!(probe.borrowed[&PID].len(), 1);
    }
    /// The other quest's flags and the delete: `pc.getf`/`pc.setf` name `quest.flag` of
    /// another quest (`G/questlua_pc.cpp:1123-1165`), and `pc.delqf` deletes a flag of the
    /// quest that runs (`:1231-1243`). Reads miss as 0; anything but text or a number
    /// logs and changes nothing.
    #[test]
    fn the_other_quest_flags_and_the_delete_follow_legacy() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let table = Value::Table(lua.create_table().unwrap());
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let get = |probe: &mut Probe, args: &[Value]| probe.call("pc.getf", args).unwrap();
        assert_eq!(get(&mut probe, &[string(b"r"), string(b"a")]), number(0.0));
        let set = |probe: &mut Probe, args: &[Value]| probe.call("pc.setf", args).unwrap();
        assert!(set(
            &mut probe,
            &[string(b"r"), string(b"a"), Value::Number(9.0)]
        )
        .is_empty());
        assert_eq!(get(&mut probe, &[string(b"r"), string(b"a")]), number(9.0));
        assert!(set(
            &mut probe,
            &[string(b"r"), string(b"a"), Value::Number(0.4)]
        )
        .is_empty());
        assert_eq!(
            get(&mut probe, &[string(b"r"), string(b"a")]),
            number(0.0),
            "a zero deletes"
        );
        assert!(set(
            &mut probe,
            &[table.clone(), string(b"a"), Value::Number(1.0)]
        )
        .is_empty());
        assert!(set(
            &mut probe,
            &[string(b"r"), table.clone(), Value::Number(1.0)]
        )
        .is_empty());
        assert!(set(&mut probe, &[string(b"r"), string(b"a"), table.clone()]).is_empty());
        assert_eq!(probe.borrowed[&PID].len(), 0, "no bad write lands");
        assert!(get(&mut probe, &[table.clone(), string(b"a")]).is_empty());
        probe
            .call("pc.setqf", &[string(b"gone"), Value::Number(3.0)])
            .unwrap();
        assert_eq!(probe.borrowed[&PID].get(b"q.gone".as_slice()), Some(&3));
        assert!(probe
            .call("pc.delqf", &[string(b"gone")])
            .unwrap()
            .is_empty());
        assert_eq!(probe.borrowed[&PID].get(b"q.gone".as_slice()), None);
        assert!(probe
            .call("pc.delqf", &[string(b"never")])
            .unwrap()
            .is_empty());
        assert!(probe.call("pc.delqf", &[table]).unwrap().is_empty());
    }

    /// `find_npc_by_vnum` (`G/questlua_global.cpp:792-823`): a number or numeric text names
    /// the race, read as a `DWORD`, and a race with no spawned NPC, or no race, answers 0.
    #[test]
    fn find_npc_by_vnum_dwords_the_race_and_misses_as_zero() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let table = Value::Table(lua.create_table().unwrap());
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        probe.game.npcs.insert(60003, 0x8000_0007);
        let find =
            |probe: &mut Probe, args: &[Value]| probe.call("find_npc_by_vnum", args).unwrap();
        assert_eq!(find(&mut probe, &number(60003.9)), number(2_147_483_655.0));
        assert_eq!(
            find(&mut probe, &[string(b"60003")]),
            number(2_147_483_655.0)
        );
        assert_eq!(
            find(&mut probe, &number(60003.0 + 4_294_967_296.0)),
            number(2_147_483_655.0),
            "the race is dworded"
        );
        assert_eq!(find(&mut probe, &number(20433.0)), number(0.0));
        assert_eq!(find(&mut probe, &[table.clone()]), number(0.0));
        assert_eq!(find(&mut probe, &[]), number(0.0));
    }

    /// `target.vid`, `target.npc` and `target.pc` (`G/questlua_target.cpp:47-89`): a target
    /// needs a name and a VID, read as a `DWORD`; the description is text or a number, and
    /// the send flag a number cast to `int`, 1 without one. The probe's quest `q` has no
    /// number, so every call is for quest 0.
    #[test]
    fn the_target_names_follow_legacys_argument_rules() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let table = Value::Table(lua.create_table().unwrap());
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        let target = |name: &[u8], vid: u32, desc: Option<&[u8]>, send: bool| Target {
            quest: 0,
            name: name.to_vec(),
            vid,
            desc: desc.map(<[u8]>::to_vec),
            send,
        };
        let vid = Value::Number(2_147_483_655.0);
        for name in ["target.vid", "target.npc", "target.pc"] {
            let made = probe
                .call(name, &[string(b"t"), vid.clone(), string(b"T")])
                .unwrap();
            assert!(made.is_empty());
            assert_eq!(
                probe.game.created.pop(),
                Some(target(b"t", 0x8000_0007, Some(b"T"), true))
            );
        }
        let calls: [(&[Value], Target); 6] = [
            (
                &[Value::Number(5.0), string(b"12"), Value::Number(3.5)],
                target(b"5", 12, Some(b"3.5"), true),
            ),
            (
                &[
                    string(b"t\0x"),
                    Value::Number(12.9),
                    table.clone(),
                    Value::Number(0.9),
                ],
                target(b"t", 12, None, false),
            ),
            (
                &[
                    string(b"t"),
                    Value::Number(12.0),
                    Value::Nil,
                    Value::Number(-1.0),
                ],
                target(b"t", 12, None, true),
            ),
            (
                &[
                    string(b"t"),
                    Value::Number(12.0),
                    Value::Nil,
                    Value::Number(f64::NAN),
                ],
                target(b"t", 12, None, true),
            ),
            (
                &[string(b"t"), Value::Number(12.0), Value::Nil, string(b"0")],
                target(b"t", 12, None, false),
            ),
            (
                &[string(b"t"), Value::Number(-1.0), Value::Nil, table.clone()],
                target(b"t", u32::MAX, None, true),
            ),
        ];
        for (args, want) in calls {
            assert!(probe.call("target.vid", args).unwrap().is_empty());
            assert_eq!(probe.game.created.pop(), Some(want));
        }
        for args in [
            vec![string(b"t")],
            vec![table.clone(), Value::Number(12.0)],
            vec![string(b"t"), table.clone()],
            vec![Value::Nil, Value::Number(12.0)],
        ] {
            assert!(probe.call("target.vid", &args).unwrap().is_empty());
            assert!(probe.game.created.is_empty(), "{args:?}");
        }
    }

    /// `target.id`, `target.delete` and `target.clear` (`G/questlua_target.cpp:91-134`): the
    /// ID answers a named target's VID, else 0; `target.delete` needs a name, and
    /// `target.clear` names none. `target.pos` stays refused.
    #[test]
    fn the_target_id_delete_and_clear_find_the_quests_targets() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let table = Value::Table(lua.create_table().unwrap());
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            borrowed: BTreeMap::new(),
            current: PID,
            stack: Vec::new(),
            window: Window::default(),
        };
        for (name, vid) in [(string(b"t"), -1.0), (Value::Number(5.0), 12.0)] {
            probe
                .call("target.vid", &[name, Value::Number(vid)])
                .unwrap();
        }
        let id = |probe: &mut Probe, args: &[Value]| probe.call("target.id", args).unwrap();
        assert_eq!(id(&mut probe, &[string(b"t")]), number(4_294_967_295.0));
        assert_eq!(id(&mut probe, &[Value::Number(5.0)]), number(12.0));
        assert_eq!(id(&mut probe, &[string(b"none")]), number(0.0));
        assert_eq!(id(&mut probe, &[table.clone()]), number(0.0));

        assert!(probe
            .call("target.delete", &[table.clone()])
            .unwrap()
            .is_empty());
        assert!(probe.game.deleted.is_empty(), "no name, no delete");
        probe.call("target.delete", &[string(b"t\0x")]).unwrap();
        assert_eq!(probe.game.deleted.pop(), Some((0, Some(b"t".to_vec()))));
        assert_eq!(id(&mut probe, &[string(b"t")]), number(0.0));
        probe.call("target.clear", &[string(b"5")]).unwrap();
        assert_eq!(probe.game.deleted.pop(), Some((0, None)));
        assert!(probe.game.targets.is_empty());
        let error = probe.call("target.pos", &[]).unwrap_err().to_string();
        assert!(error.contains("not ported: target.pos"), "{error}");
    }

    /// `NPC::OnTarget` (`G/questnpc.cpp:206-264`) under a click and `CQuestManager::Target`:
    /// in entry order at the quest's state, the first `target` entry whose argument begins
    /// with the mark's name, names the verb after its first dot and whose condition holds
    /// runs; the NPC's own scripts then do not. An unmarked NPC, a mark whose quest has no
    /// entry for it at its state, or a name past the argument's first line (`getline`) leaves
    /// the click to the NPC. The empty `questlib.lua` has no `wait`, so the script yields as
    /// `wait` does.
    #[test]
    fn a_marked_npcs_click_runs_its_quests_target_script_first() {
        const MARK: &str = r#"
quest mark begin
    state start begin
        when 20001.click begin chat("start click") end
    end
    state hunt begin
        when boss.target.arrive begin chat("boss arrive") end
        when boss.target.click with pc.get_level() > 50 begin chat("boss high") end
        when boss.target.click begin
            chat("boss click")
            q.yield("wait")
            chat("boss after")
        end
        when bossy.target.click begin chat("bossy click") end
        when "two\nlines".target.click begin chat("two lines") end
        when 20001.click begin chat("npc click") end
    end
end
"#;
        let mut manager = synthetic("mark", MARK);
        let states = manager.host().quests()[0].states.clone();
        let mut game = Fake {
            level: 10,
            ..Fake::default()
        };
        let chats = |game: &mut Fake| -> Vec<String> {
            game.chats
                .drain(..)
                .map(|(_, text)| String::from_utf8_lossy(&text).into_owned())
                .collect()
        };
        let marks = [
            (&b"boss"[..], 11),
            (b"bos", 12),
            (b"bossy", 13),
            (b"two\nlines", 15),
        ];
        for (name, vid) in marks {
            game.targets.push(Target {
                quest: 1,
                name: name.to_vec(),
                vid,
                desc: None,
                send: true,
            });
        }
        manager.load_flags(PID, [(b"mark.__status".to_vec(), states[&b"hunt"[..]])]);

        assert!(manager.click(PID, 11, 20001, &mut game));
        assert_eq!(
            chats(&mut game),
            ["boss click"],
            "the verb and the condition"
        );
        assert!(
            manager.click(PID, 11, 20001, &mut game),
            "a waiting script still takes the click"
        );
        let seen = chats(&mut game);
        assert!(seen.is_empty(), "and nothing else starts: {seen:?}");
        manager.answer(PID, 254, &mut game);
        assert_eq!(chats(&mut game), ["boss after"]);

        assert!(manager.click(PID, 12, 20001, &mut game));
        manager.answer(PID, 254, &mut game);
        assert_eq!(
            chats(&mut game),
            ["boss click", "boss after"],
            "`bos` begins `boss.click`"
        );
        assert!(manager.click(PID, 13, 20001, &mut game));
        assert_eq!(chats(&mut game), ["bossy click"]);
        game.level = 60;
        assert!(manager.click(PID, 11, 20001, &mut game));
        assert_eq!(chats(&mut game), ["boss high"]);
        assert!(manager.click(PID, 14, 20001, &mut game));
        assert_eq!(chats(&mut game), ["npc click"], "an unmarked NPC");
        assert!(manager.click(PID, 15, 20001, &mut game));
        assert_eq!(
            chats(&mut game),
            ["npc click"],
            "the argument's first line is `two`"
        );

        assert!(manager.target(PID, (1, b"boss"), b"arrive", &mut game));
        assert_eq!(chats(&mut game), ["boss arrive"]);
        assert!(!manager.target(PID, (1, b"boss"), b"die", &mut game));
        assert!(!manager.target(PID, (2, b"boss"), b"arrive", &mut game));
        assert!(!manager.target(PID + 1, (1, b"boss"), b"arrive", &mut game));
        assert!(chats(&mut game).is_empty());

        manager.load_flags(PID, [(b"mark.__status".to_vec(), states[&b"start"[..]])]);
        assert!(manager.click(PID, 11, 20001, &mut game));
        assert_eq!(
            chats(&mut game),
            ["start click"],
            "`start` has no target entry, so the NPC takes the click"
        );
    }

    /// `send_letter` (`quest/questlib.lua:142`) at login: from level 30 `item_attr67add`'s
    /// letter sends its button in skin 0 and then, as `EndRunning` does, the quest window its
    /// `q.set_title` titled (`SendQuestInfoPakcet`, `G/questpc.cpp:231-308`); its `q.start`
    /// begins nothing, because the quest has its number. The window's `info` runs the quest's
    /// `button or info` script at its state, which moves it to `talk`, where no `info`
    /// script waits, and a quest the character never ran runs nothing.
    #[test]
    fn the_letter_titles_its_quest_and_the_window_runs_its_info() {
        const INFO: &str = "It is possible to add a 6th or 7th bonus to an[ENTER]equipment \
                            item which already has 5 bonuses.[ENTER]Seon-Hae is the expert in \
                            these matters. Go and[ENTER]talk to him if you are \
                            interested.[ENTER][DONE]";
        let mut manager = manager().lock().unwrap();
        let quest = manager
            .host()
            .quest_index(b"item_attr67add")
            .expect("the bonus quest");
        let index = u16::try_from(quest).expect("a small number");
        let talk = manager.host().quests()[usize::try_from(quest).expect("small") - 1].states
            [&b"talk"[..]];
        manager.disconnect(PID);
        manager.load_flags(PID, Vec::new());
        let mut young = Fake {
            level: 29,
            map_index: 1,
            ..Fake::default()
        };
        assert!(manager.login(PID, &mut young));
        assert!(young.infos.is_empty(), "no letter below level 30");

        manager.disconnect(PID);
        manager.load_flags(PID, Vec::new());
        let mut game = Fake {
            level: 30,
            map_index: 1,
            ..Fake::default()
        };
        assert!(manager.login(PID, &mut game));
        let button = format!("[QUESTBUTTON idx;{quest}|name;Sixth and Seventh Bonus][DONE]");
        assert_eq!(game.take(), [(QUEST_SKIN_NOWINDOW, button)]);
        assert_eq!(
            game.infos,
            [QuestInfo {
                index,
                category: 0,
                begin: None,
                title: Some(b"Sixth and Seventh Bonus".to_vec()),
                clock_name: None,
                clock_value: None,
                counter_name: None,
                counter_value: None,
            }]
        );

        // While a menu waits the window's entry runs nothing (`pc.IsRunning()`,
        // `G/questnpc.cpp:765-777`), and the menu stays.
        let mut ox = Fake {
            level: 30,
            map_index: 1,
            ..Fake::at_ox_manager()
        };
        assert!(manager.click(PID, VID, OX_MANAGER, &mut ox));
        assert_eq!(ox.take().len(), 1, "the menu");
        assert!(!manager.info(PID, quest, &mut ox), "a script waits");
        assert!(ox.take().is_empty() && manager.is_running(PID));
        manager.answer(PID, 1, &mut ox);
        assert_eq!(ox.take(), [(QUEST_SKIN_NOWINDOW, "[DONE]".to_owned())]);
        assert!(!manager.is_running(PID));

        assert!(manager.info(PID, quest, &mut game), "the window asks");
        assert_eq!(game.take(), [(QUEST_SKIN_NORMAL, INFO.to_owned())]);
        assert_eq!(game.infos.len(), 1, "the info script sets no title");
        assert_eq!(
            manager.quest_flag(PID, b"item_attr67add.__status"),
            Some(talk)
        );
        assert!(!manager.info(PID, quest, &mut game), "talk has no info");
        assert!(!manager.info(PID, 999, &mut game), "no quest 999 ran");
        // `dungeoninfo` ran at the login and has a `button` but no `info`, so its entry runs
        // nothing (`QUEST no info event`), where its button would ask for input.
        let dungeoninfo = manager
            .host()
            .quest_index(b"dungeoninfo")
            .expect("the dungeon quest");
        assert!(!manager.info(PID, dungeoninfo, &mut game), "no info event");
        assert!(game.take().is_empty() && game.infos.len() == 1);
        manager.disconnect(PID);
    }

    /// `SendQuestInfoPakcet` (`G/questpc.cpp:231-308`): nothing is sent unless a bit is
    /// set; `qi.index` is the low `WORD` of `iIndex`, the category is looked up by that
    /// `WORD` (`GetQuestCategoryByQuestIndex`, `G/questmanager.cpp:312-318`) and narrowed
    /// to `c_index`'s `WORD`, and only the parts whose bits are set are carried.
    #[test]
    fn the_window_sends_the_parts_its_bits_name_under_the_words_index() {
        let categories = [(40, 7), (41, 0x0003_0009)].into_iter().collect();
        let quiet = Window {
            index: 40,
            start: true,
            title: b"kept".to_vec(),
            ..Window::default()
        };
        assert_eq!(quiet.info(&categories), None, "no bit, no record");

        let titled = Window {
            index: 0x0001_0028,
            title: b"Sixth".to_vec(),
            send: SendFlags(SendFlags::TITLE),
            ..Window::default()
        };
        assert_eq!(
            titled.info(&categories),
            Some(QuestInfo {
                index: 40,
                category: 7,
                begin: None,
                title: Some(b"Sixth".to_vec()),
                clock_name: None,
                clock_value: None,
                counter_name: None,
                counter_value: None,
            })
        );
        let begun = Window {
            index: 41,
            start: true,
            send: SendFlags(SendFlags::BEGIN),
            ..Window::default()
        };
        assert_eq!(
            begun.info(&categories),
            Some(QuestInfo {
                index: 41,
                category: 9,
                begin: Some(true),
                title: None,
                clock_name: None,
                clock_value: None,
                counter_name: None,
                counter_value: None,
            })
        );
        let done = Window {
            index: 42,
            send: SendFlags(SendFlags::BEGIN),
            ..Window::default()
        };
        assert_eq!(
            done.info(&categories)
                .map(|info| (info.category, info.begin)),
            Some((0, Some(false))),
            "a quest no line names is category 0"
        );
    }

    /// `CQuestManager::Login` (`G/questmanager.cpp:541-556`) and `NPC::OnLogin`
    /// (`G/questnpc.cpp:353-397`): `game_option`'s login script reads the flags the load
    /// set and sends the block byte, its own `+ 6` for `block_view_equipment` kept; a
    /// character the manager never saw misses.
    #[test]
    fn a_login_restores_the_block_byte_from_the_flags_the_load_read() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut stranger = Fake::default();
        assert!(!manager.login(PID, &mut stranger));
        assert!(stranger.blocked.is_empty() && stranger.chats.is_empty());

        let mut game = Fake {
            level: 10,
            map_index: 1,
            ..Fake::default()
        };
        manager.load_flags(PID, Vec::new());
        assert!(manager.login(PID, &mut game));
        assert_eq!(game.blocked, [0], "no flag sends a zero byte");
        assert!(
            !game.chats.iter().any(|(kind, _)| *kind == ChatKind::Info),
            "no flag says nothing"
        );

        let mut game = Fake {
            level: 10,
            map_index: 1,
            ..Fake::default()
        };
        manager.load_flags(
            PID,
            [
                (b"game_option.block_whisper".to_vec(), 1),
                (b"game_option.block_view_equipment".to_vec(), 1),
                (b"game_option.block_party_invite".to_vec(), 2),
            ],
        );
        assert!(manager.login(PID, &mut game));
        assert_eq!(game.blocked, [8 + 6], "whisper's 8 and the script's 6");
        let said = format!(
            "{} {} Equipment",
            text(&manager, "gameforge.game_option._100_say"),
            text(&manager, "gameforge.game_option._160_say"),
        );
        assert!(
            game.chats
                .contains(&(ChatKind::Info, said.clone().into_bytes())),
            "{:?} lacks {said:?}",
            game.chats
        );
        manager.disconnect(PID);
    }
    #[test]
    fn a_kill_runs_party_kill_on_the_leaders_pc_not_the_killers() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        manager.disconnect(PID + 1);
        let mut game = Fake::at_ox_manager();
        // The killer exists (a click); the leader is never seen — `GetPCForce`
        // creates it for the `party_kill` half.
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        manager.answer(PID, 1, &mut game);
        game.scripts.clear();
        assert!(manager.kill(PID, 691, Some(PID + 1), &mut game));
        // `guild_create`'s only `party_kill` script sits in `run`, which neither PC
        // holds, so nothing ran — but the leader's PC now exists and the killer's
        // holds no `party_kill` trace: the routing is what this pins.
        assert!(
            manager.pcs.contains_key(&(PID + 1)),
            "the leader is force-created"
        );
        assert_eq!(manager.quest_flag(PID, b"guild_create.__status"), None);
        assert_eq!(manager.quest_flag(PID + 1, b"guild_create.__status"), None);
        // An offline leader (`None`) runs the `kill` halves only: no PC appears.
        manager.disconnect(PID + 2);
        assert!(manager.kill(PID, 691, None, &mut game));
        assert!(!manager.pcs.contains_key(&(PID + 2)));
        manager.disconnect(PID);
        manager.disconnect(PID + 1);
    }

    /// `CQuestManager::LogoutPC` then `DisconnectPC` (`G/questmanager.cpp:516-535`,
    /// `:1068-1071`): the `logout` scripts run with the record held, and only the
    /// disconnect drops it. An unseen PC runs nothing and makes no record.
    #[test]
    fn a_logout_runs_scripts_and_the_disconnect_drops_the_pc() {
        let mut manager = manager().lock().unwrap();
        manager.disconnect(PID);
        let mut game = Fake::at_ox_manager();
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        manager.answer(PID, 1, &mut game);
        assert!(manager.pcs.contains_key(&PID));
        manager.logout(PID, &mut game);
        assert!(
            manager.pcs.contains_key(&PID),
            "the record stays for the save"
        );
        manager.disconnect(PID);
        assert!(!manager.pcs.contains_key(&PID), "the disconnect drops it");
        manager.logout(PID + 9, &mut game);
        assert!(!manager.pcs.contains_key(&(PID + 9)), "no record made");
    }

    /// `CQuestManager::Letter` (`G/questmanager.cpp:502-514`) and
    /// `CQuestManager::QuestButton` (`G/questmanager.cpp:767-789`): the war join quest's
    /// letter asks the client for the join button, and its button asks the player
    /// whether to join the war.
    #[test]
    fn the_war_join_letter_asks_for_the_button_and_the_button_asks_to_join() {
        let mut manager = load();
        let quest = manager
            .host()
            .quest_index(b"guild_war_join")
            .expect("the war join quest");
        let mut game = Fake {
            map_index: 100,
            any_war: 4.0,
            ..Fake::default()
        };
        // The letter runs the named quest's start state, and the quest asks the client
        // for a button that carries the quest's number.
        assert!(manager.letter(PID, quest, 0, &mut game), "the letter runs");
        let scripts = game.take();
        assert!(
            scripts
                .iter()
                .any(|(_, text)| text.contains(&format!("[QUESTBUTTON idx;{quest}|name;"))),
            "the letter asks for the button: {scripts:?}"
        );
        // The button runs the same quest, which asks the player to confirm.
        assert!(manager.button(PID, quest, &mut game), "the button runs");
        let scripts = game.take();
        assert!(
            scripts.iter().any(|(_, text)| text.contains("Knights")),
            "the button names the enemy guild: {scripts:?}"
        );
    }

    /// `CInputDB::QuestLoad` (`G/input_db.cpp:1079-1101`) with `quest::PC::SetFlag`
    /// (`G/questpc.cpp:55-72`) and `quest::PC::Build` (`G/questpc.cpp:664-679`): the
    /// stored flags come back with the character, and the quest's state is rebuilt from
    /// the flag that names it, so a quest that ran in an earlier session continues where
    /// it left it.
    #[test]
    fn the_stored_flags_come_back_with_the_character() {
        let mut manager = load();
        manager.load_flags(
            PID,
            [
                (b"guild_manage.new_withdraw_time".to_vec(), 1_700_000_000),
                (b"guild_war_join.__status".to_vec(), 1),
            ],
        );
        assert_eq!(
            manager.quest_flag(PID, b"guild_manage.new_withdraw_time"),
            Some(1_700_000_000),
            "the guild's withdraw time is the flag the guild's ladder reads"
        );
        assert_eq!(
            manager.quest_flag(PID, b"guild_war_join.__status"),
            Some(1),
            "the quest's state is the flag that names it"
        );
        // The flags a save writes are the non-zero ones, in name order.
        assert_eq!(
            manager.flags_of(PID),
            vec![
                (b"guild_manage.new_withdraw_time".to_vec(), 1_700_000_000),
                (b"guild_war_join.__status".to_vec(), 1),
            ]
        );
        // A flag whose value is zero is no flag: legacy's `SetFlag` deletes it.
        manager.load_flags(
            PID,
            [
                (b"guild_manage.new_withdraw_time".to_vec(), 1_700_000_000),
                (b"guild_war_join.__status".to_vec(), 0),
            ],
        );
        assert_eq!(manager.quest_flag(PID, b"guild_war_join.__status"), None);
        assert_eq!(
            manager.flags_of(PID),
            vec![(b"guild_manage.new_withdraw_time".to_vec(), 1_700_000_000)],
            "a zero flag is not written"
        );
        // The load sets the flags on a record that holds none, so a load replaces the
        // record's flags: the character's flags are the stored ones.
        manager.load_flags(PID, [(b"guild_war_join.__status".to_vec(), 1)]);
        assert_eq!(
            manager.flags_of(PID),
            vec![(b"guild_war_join.__status".to_vec(), 1)],
            "the load replaces what the record held"
        );
    }

    /// `quest::PC::SetFlag` (`G/questpc.cpp:52-76`) and `quest::PC::DeleteFlag`
    /// (`G/questpc.cpp:78-90`): a flag whose value is zero is no flag at all. The legacy
    /// erases it and writes the deletion, so the flag reads as absent and a save does
    /// not write it.
    #[test]
    fn a_flag_whose_value_is_zero_is_deleted() {
        let mut manager = load();
        manager.set_quest_flag(PID, b"war.is_war_member", 1);
        assert_eq!(
            manager.quest_flag(PID, b"war.is_war_member"),
            Some(1),
            "the flag is set"
        );
        manager.set_quest_flag(PID, b"war.is_war_member", 0);
        assert_eq!(
            manager.quest_flag(PID, b"war.is_war_member"),
            None,
            "the flag is gone, and not a flag of the value zero"
        );
        assert!(
            manager.flags_of(PID).is_empty(),
            "a flag that was cleared is not written"
        );
    }

    /// The quest's state is rebuilt from the flag that names it (`quest::PC::Build`,
    /// `G/questpc.cpp:664-679`): the button runs the script of the state the character
    /// is in, and a state the quest does not have has no script.
    #[test]
    fn the_quest_state_is_rebuilt_from_its_flag() {
        let mut manager = load();
        let quest = manager
            .host()
            .quest_index(b"guild_war_join")
            .expect("the war join quest");
        let mut game = Fake {
            map_index: 100,
            any_war: 4.0,
            ..Fake::default()
        };
        // The character is in the quest's start state: the button asks to join the war.
        assert!(manager.button(PID, quest, &mut game), "the button runs");
        assert!(
            game.take().iter().any(|(_, text)| text.contains("Knights")),
            "the button names the enemy guild"
        );
        // A state the quest does not have has no script, so the button runs nothing.
        manager.load_flags(PID, [(b"guild_war_join.__status".to_vec(), 1)]);
        assert!(
            !manager.button(PID, quest, &mut game),
            "a state the quest does not have has no script"
        );
        assert!(game.take().is_empty(), "nothing ran");
    }

    /// The letter and the button name one quest, where the generic event handler runs
    /// every quest of the NPC.
    #[test]
    fn a_letter_that_names_no_quest_runs_nothing() {
        let mut manager = load();
        let mut game = Fake::default();
        assert!(!manager.letter(PID, 9999, 0, &mut game), "no such quest");
        assert!(game.take().is_empty(), "nothing ran");
    }

    /// `CQuestManager::RequestEventQuest` (`G/questmanager.cpp:1956-1963`): the named
    /// quest's button runs, and an unknown name runs nothing.
    #[test]
    fn the_event_panel_runs_the_named_quests_button() {
        let mut manager = load();
        let mut game = Fake {
            map_index: 100,
            any_war: 4.0,
            ..Fake::default()
        };
        assert!(
            manager.event_quest(PID, b"guild_war_join", &mut game),
            "the named button runs"
        );
        assert!(
            game.take().iter().any(|(_, text)| text.contains("Knights")),
            "the button names the enemy guild"
        );
        assert!(
            !manager.event_quest(PID, b"no_such_quest", &mut game),
            "an unknown name runs nothing"
        );
        assert!(game.take().is_empty(), "nothing ran");
    }

    #[test]
    fn a_dword_is_the_low_half_of_the_64_bit_conversion() {
        assert_eq!(dword(7.9), 7);
        assert_eq!(dword(-1.0), u32::MAX);
        assert_eq!(dword(4_294_967_296.0 + 5.0), 5);
        assert_eq!(dword(9.3e18), 0);
        assert_eq!(dword(f64::INFINITY), 0);
        assert_eq!(dword(f64::NAN), 0);
    }
}
