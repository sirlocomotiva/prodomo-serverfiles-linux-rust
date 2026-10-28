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
//!   quest window asks for.
//!
//! While the manager runs, the [`crate::host::BRIDGED`] names reach the game through
//! [`crate::host::BRIDGE`] and a [`Game`] the caller lends for that one call.
//!
//! # Divergences
//!
//! - The dialog text, its skin and the error flag live for one call into the manager. Legacy
//!   kept them process-wide, so text a script added and never sent went out with the next
//!   character's dialog; the Rewrite logs and drops it.
//! - A script error, a suspension other than `select`, `wait` and `input` (`confirm` and
//!   `select_item` are not ported), a first result that is not text (legacy's `strcmp(NULL)`)
//!   and a `select` whose last result is not a table (legacy's unprotected `luaL_getn`) all end
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
//!   reads them (ledger 257) and `pc.learn_grand_master_skill` writes its read count; no other
//!   API reaches them yet.
//! - `getnpcid` with no name answers 0 (legacy built a `std::string` from `NULL`).
//! - An NPC's race is looked up by the caller, which finds the NPC on the character's own map.
//! - A `questcategory.txt` line with fewer than two words is logged and skipped; legacy read
//!   past the end of its word list.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use mlua::{Function, Lua, MultiValue, Thread, Value};

use crate::host::{self, Host, HostError, BRIDGE};

/// `QUEST_SKIN_NOWINDOW`: the client closes the dialog.
pub const QUEST_SKIN_NOWINDOW: u8 = 0;

/// `QUEST_SKIN_NORMAL`: the dialog window, the skin every dialog starts with.
pub const QUEST_SKIN_NORMAL: u8 = 1;

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

    /// The race of the character `vid`, when it is in the world.
    fn race_of(&self, vid: u32) -> Option<u32>;

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

    /// `LearnGrandMasterSkill(vnum)` (`G/char_skill.cpp:341`): the 7/8th-skills
    /// quest's read of a GRAND skill (230g). `total` is the read count including
    /// this one, which the bridge already stamped in the quest flags. Answers
    /// whether the skill climbed.
    fn learn_grand_master_skill(&mut self, vnum: u32, total: i32) -> bool;

    /// `GetSkillLevel(vnum)` of the character the script runs for (230g).
    fn skill_level(&self, vnum: u32) -> u8;

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

    /// One `guild.*` quest call (230s): the call's member name (`level`, `get_sp0`) and
    /// its arguments, dworded numbers or raw text. The world's guilds judge it now, while
    /// the script that asked still waits, and answer what legacy pushed.
    fn guild_call(&mut self, name: &[u8], args: &[GuildArg]) -> GuildAnswer;

    /// Sends the character a `GC_QUEST_INFO`: the window of the quest whose script just
    /// ended (`quest::PC::SendQuestInfoPakcet`, `G/questpc.cpp:231-308`).
    fn quest_info(&mut self, info: &QuestInfo);
}

/// What `SendQuestInfoPakcet` (`G/questpc.cpp:231-308`) sends when a script that changed its
/// quest's window ends: the quest's number and category, and the parts the script changed.
/// The clock, the counter and the icon are not ported, so no script changes them.
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
        }
    }

    /// The Lua state and the loaded scripts.
    pub fn host(&self) -> &Host {
        &self.host
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

    /// `game.set_event_flag`: sets an event flag, which `noguard` reads. A GM command
    /// would call this; no quest on the owner's data sets `noguard`.
    pub fn set_event_flag(&mut self, name: &[u8], value: i32) {
        self.event_flags.insert(name.to_vec(), value);
    }

    /// `CQuestManager::Click`: character `pid` clicked the NPC `npc_vid` of race `race`. Returns
    /// whether a quest took the click, in which case the NPC's shop does not open.
    pub fn click(&mut self, pid: u32, npc_vid: u32, race: u32, game: &mut dyn Game) -> bool {
        // `CHARACTER::OnClick` records the NPC before it asks the quests (`SetQuestNPCID`).
        self.pcs.entry(pid).or_default().npc_vid = npc_vid;
        self.operate(pid, game, |run, pc, npcs| {
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

    /// `CQuestManager::DisconnectPC`: the character logged out, and its running script ends
    /// unfinished.
    pub fn logout(&mut self, pid: u32) {
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
        } = self;
        let pc = pcs.entry(pid).or_default();
        // `SetQuestNPCID` overwrites what the click recorded (`Reward` runs after any
        // click, and the kill's scripts read the victim).
        if *quest_npc_vid != 0 {
            pc.npc_vid = *quest_npc_vid;
            *quest_npc_vid = 0;
        }
        // The bridge owns `pc.flags` while Lua runs (`Cx::quest_flags`): a name whose
        // answer mutates quest state does it now, while the script that asked still
        // waits. Swapped out and back — `execute` also writes `__status` there, but
        // never while a bridge call holds the `RefCell`.
        let mut loaned = std::mem::take(&mut pc.flags);
        let host: &Host = host;
        let quest_indices = host.quest_indices();
        let cx = RefCell::new(Cx {
            output: Output::default(),
            game,
            event_flags,
            npc_ids,
            quest_indices,
            npc_vid: pc.npc_vid,
            quest: pc.current.clone(),
            pending_state: None,
            quest_flags: Some(&mut loaned),
            window: Window::default(),
            categories,
        });
        let run = Run { host, cx: &cx };
        let result = run.bridged(|| op(&run, pc, npcs));
        let cx = cx.into_inner();
        let unsent = cx.output.script.clone();
        // The loaned map is the live one: the bridge's writes landed there, and
        // `pc.flags` stood empty meanwhile — except `execute`'s `__status`, which
        // wrote the empty map and is merged back on top.
        for (name, value) in std::mem::take(&mut pc.flags) {
            loaned.insert(name, value);
        }
        pc.flags = loaned;
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
    npc_vid: u32,
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
#[derive(Debug, Default)]
struct Window {
    /// `iIndex`: `PC::SetQuest` gives it the number of the quest that runs.
    index: u32,
    /// `bStart`.
    start: bool,
    /// `_title`.
    title: Vec<u8>,
    /// `QUEST_SEND_ISBEGIN`: `q.start` or `q.done` moved `bStart`.
    send_begin: bool,
    /// `QUEST_SEND_TITLE`: `q.set_title` set the title.
    send_title: bool,
}

impl Window {
    /// What `SendQuestInfoPakcet` sends, when the script changed anything
    /// (`if (m_iSendToClient)`, `G/questpc.cpp:342-346`). The category is looked up by the
    /// index the record carries.
    fn info(&self, categories: &BTreeMap<u16, u32>) -> Option<QuestInfo> {
        if !self.send_begin && !self.send_title {
            return None;
        }
        let [low, high, ..] = self.index.to_le_bytes();
        let index = u16::from_le_bytes([low, high]);
        let [low, high, ..] = categories.get(&index).copied().unwrap_or(0).to_le_bytes();
        Some(QuestInfo {
            index,
            category: u16::from_le_bytes([low, high]),
            begin: self.send_begin.then_some(self.start),
            title: self.send_title.then(|| self.title.clone()),
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
    /// The quest that runs, for error messages (`GetCurrentQuestName`).
    quest: Vec<u8>,
    /// What `set_state` asked for in this step, which [`Run::end_running`] moves the `Pc` to
    /// when the script ends, or the `Pc` carries while the script waits.
    pending_state: Option<Vec<u8>>,
    /// The character's quest flags: the split half the bridge owns while `op` keeps
    /// the rest (`operate`). A name whose answer mutates quest state
    /// (`pc.learn_grand_master_skill`'s count) does it here, now. `None` only in
    /// the direct-call probe, which answers through the `Game` alone.
    quest_flags: Option<&'g mut BTreeMap<Vec<u8>, i32>>,
    /// The quest window of the script whose step runs, which `q.set_title`, `q.start` and
    /// `q.done` change; [`Run::resume`] moves it in from [`Running::window`] and back.
    window: Window,
    /// Each quest's category (`GetQuestCategoryByQuestIndex`).
    categories: &'g BTreeMap<u16, u32>,
}

/// One call into the manager. No `Cx` borrow is held across a call into Lua, which may call
/// back through the bridge.
struct Run<'r, 'g> {
    host: &'r Host,
    cx: &'r RefCell<Cx<'g>>,
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
    /// (`quest_setstate` writes the running state at once, `G/questlua_quest.cpp:112-146`).
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
            self.goto_state(pc, running.quest, &state);
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
            b"confirm" | b"select_item" => {
                tracing::warn!(
                    "QUEST not ported: {} suspends the script",
                    String::from_utf8_lossy(&what)
                );
                None
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
    /// `SetCurrentQuestStateName` (`questlua_quest.cpp:112`): the quest of the finished script
    /// moves to the named state. A name the quest does not have is legacy's index 0
    /// (`GetQuestStateIndex`), and the status flag follows the state.
    fn goto_state(&self, pc: &mut Pc, quest: u32, state: &[u8]) {
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
        pc.flags.insert([&name[..], b".__status"].concat(), number);
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

/// `(DWORD)lua_tonumber(L, n)` on x86-64: the fraction dropped, then the low 32 bits of the
/// 64-bit conversion, which is 0 where the conversion has no answer (out of range or NaN).
fn dword(number: f64) -> u32 {
    let truncated = number.trunc();
    if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&truncated) {
        return 0;
    }
    let wide: i64 = format!("{truncated:.0}").parse().unwrap_or(0);
    let bytes = wide.to_le_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
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

/// The `pc.*` bridged names: the level, the map, the quest flags, the grand-master read, the
/// skill level and the block mode. Returns `None` for a name this table does not hold.
fn pc_api(
    lua: &Lua,
    cx: &mut Cx<'_>,
    name: &[u8],
    (first, last): (&Value, &Value),
) -> mlua::Result<Option<MultiValue>> {
    let number = |value: u32| Some(Value::Number(f64::from(value)));
    let result = match name {
        b"pc.get_level" => number(u32::from(cx.game.level())),
        b"pc.get_map_index" => Some(Value::Number(f64::from(cx.game.map_index()))),
        b"pc.get_war_map" => Some(Value::Number(f64::from(cx.game.war_map()))),
        b"pc.is_gm" => Some(Value::Boolean(false)),
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
                .quest_flags
                .as_deref()
                .and_then(|flags| flags.get(&name).copied())
                .unwrap_or(0);
            Some(Value::Number(f64::from(value)))
        }
        b"pc.learn_grand_master_skill" => {
            // `pc_learn_grand_master_skill` (`questlua_pc.cpp:121-133`): a non-number
            // pushes nothing; a number pushes the climb. The count lives here, in
            // the character's flags, so the second read in one script sees the first.
            let Some(vnum) = lua.coerce_number(first.clone())? else {
                return Ok(Some(MultiValue::new()));
            };
            let vnum = dword(vnum);
            let name = format!("training_grandmaster_skill.skill{vnum}");
            let total = cx
                .quest_flags
                .as_deref()
                .and_then(|flags| flags.get(name.as_bytes()).copied())
                .unwrap_or(0)
                + 1;
            if let Some(flags) = cx.quest_flags.as_deref_mut() {
                flags.insert(name.into_bytes(), total);
            }
            let climbed = cx.game.learn_grand_master_skill(vnum, total);
            Some(Value::Boolean(climbed))
        }
        b"pc.get_skill_level" => {
            // `pc_get_skill_level` (`questlua_pc.cpp:1734-1749`): a non-number logs
            // and answers 0; a number answers the level.
            let Some(vnum) = lua.coerce_number(first.clone())? else {
                tracing::warn!("QUEST invalid argument");
                return Ok(Some(MultiValue::from_iter([Value::Number(0.0)])));
            };
            Some(Value::Number(f64::from(cx.game.skill_level(dword(vnum)))))
        }
        b"pc.send_block_mode" => {
            // `pc_send_block_mode` (`G/questlua_pc.cpp:2190-2197`): the number
            // becomes the character's own block mode; a non-number is zero
            // (`(BYTE)lua_tonumber`), and only the low byte survives.
            let mode = match lua.coerce_number(first.clone())? {
                Some(number) => {
                    let bytes = dword(number).to_le_bytes();
                    bytes[0]
                }
                None => 0,
            };
            cx.game.send_block_mode(mode);
            None
        }
        _ => return Ok(None),
    };
    Ok(Some(result.into_iter().collect()))
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
    };
    Ok(Some(pushed))
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
                cx.window.send_title = true;
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
                cx.window.send_begin = true;
            }
        }
        b"q.done" => {
            // `SetCurrentQuestDoneFlag` (`G/questpc.cpp:223-229`) with
            // `ClearCurrentQuestBeginFlag` (`:433-439`): a quest with a number ends.
            if cx.window.index != 0 {
                cx.window.start = false;
                cx.window.send_begin = true;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn api(lua: &Lua, cx: &mut Cx<'_>, name: &[u8], args: &MultiValue) -> mlua::Result<MultiValue> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let last = args.back().cloned().unwrap_or(Value::Nil);
    let number = |value: u32| Some(Value::Number(f64::from(value)));
    if let TextOutcome::Hit(result) = text_api(lua, cx, name, args, last.clone())? {
        return Ok(result.into_iter().collect());
    }
    if let Some(answered) = pc_api(lua, cx, name, (&first, &last))? {
        return Ok(answered);
    }
    if let Some(answered) = guild_api(lua, cx, name, args)? {
        return Ok(answered);
    }
    if window_api(lua, cx, name, last.clone())? {
        return Ok(MultiValue::new());
    }
    let result = match name {
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
            None
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
            None
        }
        b"getnpcid" => {
            let id = host::c_string(lua, last)?
                .and_then(|name| cx.npc_ids.get(&name).copied())
                .unwrap_or(0);
            number(id)
        }
        b"number" => {
            let second = args.get(1).cloned().unwrap_or(Value::Nil);
            let both = lua.coerce_number(first.clone())?.is_some()
                && lua.coerce_number(second.clone())?.is_some();
            let drawn = if both {
                let from = host::c_int(lua, first)?;
                let to = host::c_int(lua, second)?;
                cx.game.number(from, to)
            } else {
                0
            };
            Some(Value::Number(f64::from(drawn)))
        }
        b"mob_name" => {
            let text = match lua.coerce_number(first)? {
                Some(vnum) => cx.game.mob_name(dword(vnum)).unwrap_or_default(),
                None => Vec::new(),
            };
            Some(Value::String(lua.create_string(text)?))
        }
        b"get_time" | b"get_global_time" => number(cx.game.global_time()),
        b"game.get_event_flag" => {
            let value = host::c_string(lua, first)?
                .and_then(|flag| cx.event_flags.get(&flag).copied())
                .unwrap_or(0);
            Some(Value::Number(f64::from(value)))
        }
        b"npc.getrace" | b"npc.get_race" => number(cx.game.race_of(cx.npc_vid).unwrap_or(0)),
        b"q.getcurrentquestindex" => {
            // `CQuestManager::GetQuestIndexByName` (`G/questmanager.cpp:1282-1290`): the
            // number of the quest that runs, and 0 for a name no quest has.
            let index = cx
                .quest_indices
                .get(cx.quest.as_slice())
                .copied()
                .unwrap_or(0);
            number(index)
        }
        b"q.getcurrentquestname" => Some(Value::String(lua.create_string(cx.quest.clone())?)),
        b"set_state" | b"q.setstate" | b"q.set_state" => {
            match host::c_string(lua, last)? {
                Some(state) if !state.is_empty() => {
                    cx.pending_state = Some(state);
                }
                _ => tracing::warn!("QUEST state name is empty"),
            }
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
    struct Fake {
        level: u8,
        races: BTreeMap<u32, u32>,
        chats: Vec<(ChatKind, Vec<u8>)>,
        scripts: Vec<(u8, Vec<u8>)>,
        draws: Vec<(i32, i32)>,
        learned: BTreeSet<u32>,
        guild_calls: Vec<(Vec<u8>, Vec<GuildArg>)>,
        map_index: i32,
        war_map: i32,
        any_war: f64,
        blocked: Vec<u8>,
        infos: Vec<QuestInfo>,
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

        fn race_of(&self, vid: u32) -> Option<u32> {
            self.races.get(&vid).copied()
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

        fn learn_grand_master_skill(&mut self, vnum: u32, _total: i32) -> bool {
            !self.learned.insert(vnum)
        }

        fn skill_level(&self, vnum: u32) -> u8 {
            u8::from(self.learned.contains(&vnum)) * 40
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

        fn guild_call(&mut self, name: &[u8], args: &[GuildArg]) -> GuildAnswer {
            self.guild_calls.push((name.to_vec(), args.to_vec()));
            match name {
                b"get_any_war" => GuildAnswer::Number(self.any_war),
                b"name" => GuildAnswer::Text(b"Knights".to_vec()),
                _ => GuildAnswer::None,
            }
        }

        fn quest_info(&mut self, info: &QuestInfo) {
            self.infos.push(info.clone());
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
        manager.logout(PID);
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
        manager.logout(PID);
    }

    #[test]
    fn closing_the_menu_closes_the_window() {
        let mut manager = manager().lock().unwrap();
        manager.logout(PID);
        let mut game = Fake::at_ox_manager();
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        game.take();
        // `Inchide` is entry 1, past the one quest entry.
        manager.answer(PID, 1, &mut game);
        assert_eq!(game.take(), [(0, "[DONE]".to_owned())]);
        assert!(!manager.is_running(PID));
        manager.logout(PID);
    }

    /// `NPC::HandleEvent` starts nothing for a character whose script waits
    /// (`questnpc.cpp:484`); the guild land seller's click, which starts its script, is the
    /// positive control.
    #[test]
    fn a_click_while_a_script_waits_starts_no_other() {
        const LAND_SELLER: u32 = 20_040;
        let mut manager = manager().lock().unwrap();
        manager.logout(PID);
        let mut game = Fake::at_ox_manager();
        assert!(!manager.npcs[&LAND_SELLER].has_chat());
        assert!(manager.click(PID, VID, LAND_SELLER, &mut game));
        assert!(!game.take().is_empty());
        manager.logout(PID);
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        game.take();
        assert!(!manager.click(PID, VID, LAND_SELLER, &mut game));
        assert!(game.take().is_empty());
        assert!(manager.is_running(PID));
        manager.logout(PID);
    }

    /// `input()` sends `[INPUT]` and waits for `CG_QUEST_INPUT_STRING`, which it reads up to its
    /// first NUL (`GotoInputState`, `CQuestManager::Input`); no other answer continues it, and
    /// no input continues another wait.
    #[test]
    fn an_input_waits_for_the_text_and_nothing_else() {
        let mut manager = manager().lock().unwrap();
        manager.logout(PID);
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
        manager.logout(PID);
    }

    /// `quest_setstate` writes the running state in whichever step of the script calls it
    /// (`G/questlua_quest.cpp:112-146`), and `EndRunning` moves the quest when the script
    /// ends (`G/questpc.cpp:352-371`): a `set_state` before a `wait()` lands when the answer
    /// ends the script, one after it lands too, and a refusal ends the script with the state
    /// it asked for.
    #[test]
    fn a_set_state_lands_when_the_script_ends_whichever_step_asked() {
        let mut manager = manager().lock().unwrap();
        manager.logout(PID);
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
        manager.logout(PID);
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
        manager.logout(PID);
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
        manager.logout(PID);
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
        manager.logout(PID);
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
        manager.logout(PID);
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
    /// whose window has no number yet, and `q.done` ends only one that has.
    #[test]
    fn the_quest_window_names_follow_legacys_argument_rules() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let string = |text: &[u8]| Value::String(lua.create_string(text).unwrap());
        let mut probe = Probe {
            manager: &manager,
            game: Fake::default(),
            output: Output::default(),
            flags: BTreeMap::new(),
            window: Window::default(),
        };
        let table = Value::Table(lua.create_table().unwrap());
        probe.call("q.set_title", &[table]).unwrap();
        probe.call("q.set_title", &[]).unwrap();
        assert!(!probe.window.send_title, "no text, no title");
        probe
            .call("q.set_title", &[string(b"first"), string(b"la\0st")])
            .unwrap();
        assert_eq!(probe.window.title, b"la");
        probe.call("q.set_title", &[Value::Number(12.5)]).unwrap();
        assert_eq!(probe.window.title, b"12.5");
        assert!(probe.window.send_title);

        // The probe's quest `q` has no number, so its window has none either.
        probe.call("q.done", &[]).unwrap();
        assert!(!probe.window.send_begin, "nothing to end");
        probe.call("q.start", &[]).unwrap();
        assert!(probe.window.start && probe.window.send_begin);
        assert_eq!(probe.window.index, 0, "`q` names no quest");

        probe.window = Window {
            index: 40,
            ..Window::default()
        };
        probe.call("q.start", &[]).unwrap();
        assert!(
            !probe.window.start && !probe.window.send_begin,
            "a numbered quest is never begun"
        );
        probe.call("q.done", &[]).unwrap();
        assert!(!probe.window.start && probe.window.send_begin);
        let categories = [(40, 0x0002_0003)].into_iter().collect();
        assert_eq!(
            probe.window.info(&categories),
            Some(QuestInfo {
                index: 40,
                category: 3,
                begin: Some(false),
                title: None,
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
        /// The loaned quest flags, as `operate` lends them: the count arms persist here.
        flags: BTreeMap<Vec<u8>, i32>,
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
                quest: b"q".to_vec(),
                pending_state: None,
                quest_flags: Some(&mut self.flags),
                window: std::mem::take(&mut self.window),
                categories: &self.manager.categories,
            };
            let args: MultiValue = args.iter().cloned().collect();
            let result = api(self.manager.host.lua(), &mut cx, name.as_bytes(), &args);
            self.output = cx.output;
            self.window = cx.window;
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
            flags: BTreeMap::new(),
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
        let error = probe.call("pc.warp", &[]).unwrap_err().to_string();
        assert!(error.contains("not ported: pc.warp"), "{error}");
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
            flags: BTreeMap::new(),
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
    }

    /// The `pc.*` arms: the level, the grand-master read (each numbered read bumps the
    /// count in the flags, dworded), and the skill level.
    #[test]
    fn the_pc_names_answer_and_count_the_reads() {
        let manager = manager().lock().unwrap();
        let lua = manager.host.lua();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            flags: BTreeMap::new(),
            window: Window::default(),
        };
        probe.game.level = 40;
        assert_eq!(probe.call("pc.get_level", &[]).unwrap(), number(40.0));
        let table = Value::Table(lua.create_table().unwrap());
        // a non-number pushes nothing.
        let climbed = |probe: &mut Probe, args: &[Value]| {
            probe.call("pc.learn_grand_master_skill", args).unwrap()
        };
        assert_eq!(climbed(&mut probe, &number(1.9)), [Value::Boolean(false)]);
        assert_eq!(climbed(&mut probe, &number(1.9)), [Value::Boolean(true)]);
        assert_eq!(
            probe
                .flags
                .get(b"training_grandmaster_skill.skill1".as_slice()),
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

    /// `pc_send_block_mode` (`G/questlua_pc.cpp:2190-2197`): the number becomes
    /// the character's own block mode, dworded and low-byte only; a non-number
    /// is zero; the call pushes nothing.
    #[test]
    fn the_block_mode_number_becomes_the_characters_own_byte() {
        let manager = manager().lock().unwrap();
        let number = |value: f64| vec![Value::Number(value)];
        let mut probe = Probe {
            manager: &manager,
            game: Fake::at_ox_manager(),
            output: Output::default(),
            flags: BTreeMap::new(),
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
            flags: BTreeMap::new(),
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

    #[test]
    fn a_level_up_runs_the_no_target_scripts_and_misses_an_unknown_character() {
        let mut manager = manager().lock().unwrap();
        manager.logout(PID);
        let mut stranger = Fake::default();
        assert!(!manager.levelup(PID, &mut stranger));
        assert!(stranger.scripts.is_empty());
        let mut game = Fake::at_ox_manager();
        assert!(manager.click(PID, VID, OX_MANAGER, &mut game));
        game.scripts.clear();
        manager.logout(PID);
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
        assert!(game.scripts.is_empty(), "a `set_state` sends nothing");
        manager.logout(PID);
        manager.pcs.entry(PID).or_default();
        game.level = 120;
        assert!(manager.levelup(PID, &mut game));
        assert_eq!(
            manager.quest_flag(PID, b"yohara_system.__status"),
            Some(1_998_515_313),
            "at 120 the yohara quest moves to CRC32(`information_new`)"
        );
        manager.logout(PID);
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
        manager.logout(PID);
        manager.load_flags(PID, Vec::new());
        let mut young = Fake {
            level: 29,
            map_index: 1,
            ..Fake::default()
        };
        assert!(manager.login(PID, &mut young));
        assert!(young.infos.is_empty(), "no letter below level 30");

        manager.logout(PID);
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
        manager.logout(PID);
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
            send_title: true,
            ..Window::default()
        };
        assert_eq!(
            titled.info(&categories),
            Some(QuestInfo {
                index: 40,
                category: 7,
                begin: None,
                title: Some(b"Sixth".to_vec()),
            })
        );
        let begun = Window {
            index: 41,
            start: true,
            send_begin: true,
            ..Window::default()
        };
        assert_eq!(
            begun.info(&categories),
            Some(QuestInfo {
                index: 41,
                category: 9,
                begin: Some(true),
                title: None,
            })
        );
        let done = Window {
            index: 42,
            send_begin: true,
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
        manager.logout(PID);
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
        manager.logout(PID);
    }
    #[test]
    fn a_kill_runs_party_kill_on_the_leaders_pc_not_the_killers() {
        let mut manager = manager().lock().unwrap();
        manager.logout(PID);
        manager.logout(PID + 1);
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
        manager.logout(PID + 2);
        assert!(manager.kill(PID, 691, None, &mut game));
        assert!(!manager.pcs.contains_key(&(PID + 2)));
        manager.logout(PID);
        manager.logout(PID + 1);
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
