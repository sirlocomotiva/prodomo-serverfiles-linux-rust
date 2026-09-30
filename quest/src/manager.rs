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
//!   entry or continues a `wait`, and [`Manager::input`] is `CG_QUEST_INPUT_STRING`.
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
//! - Quest states and flags live as long as the character is logged in; no API that reads them
//!   is ported yet.
//! - `getnpcid` with no name answers 0 (legacy built a `std::string` from `NULL`).
//! - An NPC's race is looked up by the caller, which finds the NPC on the character's own map.

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
}

impl Manager {
    /// Loads the [`Host`] of the locale directory `locale_dir` and indexes its scripts with
    /// `quest/questnpc.txt`.
    ///
    /// # Errors
    ///
    /// The host fails to load, or `questnpc.txt` cannot be read.
    pub fn load(locale_dir: &Path) -> Result<Manager, HostError> {
        let host = Host::load(locale_dir)?;
        let path = locale_dir.join("quest").join("questnpc.txt");
        let questnpc = fs::read(&path).map_err(|error| HostError::Io(path, error))?;
        Ok(Manager::new(host, &questnpc))
    }

    /// Indexes the scripts of `host` by NPC, with the NPC names of `questnpc`.
    pub fn new(host: Host, questnpc: &[u8]) -> Manager {
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
        Manager {
            host,
            npcs,
            npc_ids,
            event_flags,
            pcs: BTreeMap::new(),
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

    /// Whether a script of character `pid` is suspended, waiting for the client.
    pub fn is_running(&self, pid: u32) -> bool {
        self.pcs.get(&pid).is_some_and(|pc| pc.running.is_some())
    }

    /// A quest flag of character `pid`, such as `QUEST.__status`, the state a quest last ran in.
    pub fn quest_flag(&self, pid: u32, name: &[u8]) -> Option<i32> {
        self.pcs.get(&pid)?.flags.get(name).copied()
    }

    /// `game.get_event_flag`: an event flag, 0 when it was never set.
    pub fn event_flag(&self, name: &[u8]) -> i32 {
        self.event_flags.get(name).copied().unwrap_or(0)
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

    /// `CInputMain::ScriptAnswer`: an answer above 250 continues a `wait`, any other picks entry
    /// `answer` of a menu.
    pub fn answer(&mut self, pid: u32, answer: u8, game: &mut dyn Game) {
        if answer > ANSWER_RESUME {
            self.resume(pid, game);
        } else {
            self.select(pid, u32::from(answer), game);
        }
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
        } = self;
        let pc = pcs.entry(pid).or_default();
        let cx = RefCell::new(Cx {
            output: Output::default(),
            game,
            event_flags,
            npc_ids,
            npc_vid: pc.npc_vid,
            quest: pc.current.clone(),
        });
        let run = Run { host, cx: &cx };
        let result = run.bridged(|| op(&run, pc, npcs));
        let unsent = cx.into_inner().output.script;
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
    /// The NPC the character clicked last (`CHARACTER::m_dwQuestNPCVID`).
    npc_vid: u32,
}

/// A running script (`QuestState`).
#[derive(Debug)]
struct Running {
    thread: mlua::Result<Thread>,
    suspend: Suspend,
    /// `chat_scripts`: the entries of the chat menu this script shows.
    chat: Vec<ArgScript>,
    state: i32,
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
    npc_vid: u32,
    /// The quest that runs, for error messages (`GetCurrentQuestName`).
    quest: Vec<u8>,
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
        pc.quests.insert(quest, state);
        pc.flags.insert([&name[..], b".__status"].concat(), state);
        pc.send_done = false;
        self.cx.borrow_mut().quest.clone_from(&name);
        pc.current = name;
        pc.running = Some(Running {
            thread,
            suspend: Suspend::None,
            chat,
            state,
        });
        self.resume(pc, Ok(MultiValue::new()))
    }

    /// `RunState` (`questlua.cpp:995`), and the `CloseState` and `EndRunning` every caller did
    /// when it returned false.
    fn resume(&self, pc: &mut Pc, args: mlua::Result<MultiValue>) -> bool {
        let Some(running) = pc.running.as_ref() else {
            return false;
        };
        self.cx.borrow_mut().output.error = false;
        let resumed = match &running.thread {
            Ok(thread) => args.and_then(|args| thread.resume::<MultiValue>(args)),
            Err(error) => Err(error.clone()),
        };
        let step = match resumed {
            Ok(values) if values.is_empty() => {
                self.goto_end(pc);
                pc.running = None;
                return false;
            }
            Ok(values) => self.suspension(&values),
            Err(error) => {
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
            pc.running = None;
            return false;
        };
        self.cx.borrow_mut().output.script.extend(text);
        if let Some(running) = pc.running.as_mut() {
            running.suspend = suspend;
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
fn api(lua: &Lua, cx: &mut Cx<'_>, name: &[u8], args: &MultiValue) -> mlua::Result<MultiValue> {
    let first = args.front().cloned().unwrap_or(Value::Nil);
    let last = args.back().cloned().unwrap_or(Value::Nil);
    let number = |value: u32| Some(Value::Number(f64::from(value)));
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
        races: BTreeMap<u32, u32>,
        chats: Vec<(ChatKind, Vec<u8>)>,
        scripts: Vec<(u8, Vec<u8>)>,
        draws: Vec<(i32, i32)>,
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
    }

    impl Probe<'_> {
        fn call(&mut self, name: &str, args: &[Value]) -> mlua::Result<Vec<Value>> {
            let mut cx = Cx {
                output: std::mem::take(&mut self.output),
                game: &mut self.game,
                event_flags: &self.manager.event_flags,
                npc_ids: &self.manager.npc_ids,
                npc_vid: VID,
                quest: b"q".to_vec(),
            };
            let args: MultiValue = args.iter().cloned().collect();
            let result = api(self.manager.host.lua(), &mut cx, name.as_bytes(), &args);
            self.output = cx.output;
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
        assert_eq!(flag(&mut probe, &[table]), number(0.0));
        for name in ["get_time", "get_global_time"] {
            assert_eq!(probe.call(name, &[]).unwrap(), number(1_790_000_000.0));
        }
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
        let error = probe.call("pc.warp", &[]).unwrap_err().to_string();
        assert!(error.contains("not ported: pc.warp"), "{error}");
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
