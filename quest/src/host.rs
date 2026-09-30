//! The Lua 5.1 state every quest runs in (ADR-0004, ADR-0006).
//!
//! [`Host::load`] builds it the way legacy's `CQuestManager::InitializeLua` did
//! (`questlua.cpp:644-827`):
//! - the standard libraries legacy opened: base (with `coroutine`), `table`, `string`, `math`,
//!   and an `os` holding only `date`, `time`, `clock` and `difftime` (legacy's `io` and `debug`
//!   are left out, a Divergence);
//! - every name of [`crate::api`]: the load-time recorders `add_bgm_info`, `add_goto_info`,
//!   `set_bgm_volume_enable` and `arena.add_map`, which `settings.lua` calls, `q.yield`, which is
//!   `coroutine.yield`, `get_locale_base_path`, the [`BRIDGED`] names, which reach the game
//!   through [`BRIDGE`] while [`crate::manager`] runs a script, and a "not ported" refusal for
//!   the rest;
//! - [`PRELUDE`], the Lua 5.0 behaviour the sources rely on;
//! - the library chain `settings.lua`, `quest/questlib.lua`, `translate.lua` and
//!   `quest/locale.lua`, whose failure fails the load as legacy's `return false` did;
//! - every script of [`crate::sources::load_order`] compiled with [`crate::qc`], a script `qc`
//!   refuses being logged and left out, and each quest's state table run and indexed both ways
//!   (`BuildStateIndexToName`, `questlua.cpp:613`).
//!
//! Every chunk is read through [`crate::dialect::translate`]: the libraries, the state tables,
//! `dofile` and `loadstring`. `dofile` reads only below the locale directory, which
//! `get_locale_base_path()` names. The compiled `when` scripts are compiled once at load and kept
//! by their `object/` path, as legacy's `__codecache` kept them after their first run.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use mlua::{ChunkMode, Function, Lua, LuaOptions, MultiValue, StdLib, Table, Value};

use crate::api;
use crate::dialect;
use crate::qc::{Compiler, Objects, QcError};
use crate::sources;

/// The Lua 5.0 behaviour the quest sources rely on, in Lua 5.1 (ADR-0006).
///
/// - `__compat_iter`: a generic `for` over a table walks it with `next`, as Lua 5.0's
///   `OP_TFORPREP` did;
/// - `table.getn`, `setn`, `insert`, `remove` and `foreachi` as `ltablib.c` and `lauxlib.c` of
///   `server/server/liblua` wrote them: the length is a numeric field `n`, else the size `setn`
///   or `insert` recorded in a table with weak keys, else a count up to the first nil, and
///   `checkint` reads a value that is not a number as -1 and one no `int` holds as `INT_MIN`,
///   as x86's conversion does.
///
/// The chunk returns its `getn`, which [`getn`] calls for the host (`luaL_getn`).
pub const PRELUDE: &str = r##"
do
  local type, next, tonumber, rawget, rawset, error, select, setmetatable =
    type, next, tonumber, rawget, rawset, error, select, setmetatable
  local floor, ceil = math.floor, math.ceil

  function __compat_iter(f, s, var)
    if type(f) == "table" then
      return next, f, var
    end
    return f, s, var
  end

  local sizes = setmetatable({}, { __mode = "k" })

  -- `(int)` of a `double`: the fraction dropped toward zero, and `INT_MIN` where x86's
  -- conversion has no `int` (out of range or NaN).
  local function int(n)
    if n ~= n or n <= -2147483649 or n >= 2147483648 then
      return -2147483648
    end
    if n < 0 then
      return ceil(n)
    end
    return floor(n)
  end

  -- `checkint`: the value as an `int`, -1 where it is not a number.
  local function checkint(value)
    local n = tonumber(value)
    if n == nil then
      return -1
    end
    return int(n)
  end

  local function getn(t)
    local n = checkint(rawget(t, "n"))
    if n >= 0 then
      return n
    end
    n = checkint(rawget(sizes, t))
    if n >= 0 then
      return n
    end
    n = 0
    while rawget(t, n + 1) ~= nil do
      n = n + 1
    end
    return n
  end

  local function setn(t, n)
    if checkint(rawget(t, "n")) >= 0 then
      rawset(t, "n", n)
    else
      rawset(sizes, t, n)
    end
  end

  local function got(count, position, value)
    if count < position then
      return "no value"
    end
    return type(value)
  end

  local function check_table(name, count, t)
    if type(t) ~= "table" then
      error("bad argument #1 to '" .. name .. "' (table expected, got " .. got(count, 1, t)
        .. ")", 0)
    end
  end

  -- `luaL_checkint`: a number (or a string that reads as one) as an `int`.
  local function check_int(name, position, count, value)
    local n = tonumber(value)
    if n == nil then
      error("bad argument #" .. position .. " to '" .. name .. "' (number expected, got "
        .. got(count, position, value) .. ")", 0)
    end
    return int(n)
  end

  table.getn = function(...)
    local t = ...
    check_table("getn", select("#", ...), t)
    return getn(t)
  end

  table.setn = function(...)
    local count = select("#", ...)
    local t, n = ...
    check_table("setn", count, t)
    setn(t, check_int("setn", 2, count, n))
  end

  table.insert = function(...)
    local count = select("#", ...)
    local t, position, value = ...
    check_table("insert", count, t)
    local n = getn(t) + 1
    if count == 2 then
      value = position
      position = n
    else
      position = check_int("insert", 2, count, position)
      if position > n then
        n = position
      end
    end
    setn(t, n)
    n = n - 1
    while n >= position do
      rawset(t, n + 1, rawget(t, n))
      n = n - 1
    end
    rawset(t, position, value)
  end

  table.remove = function(...)
    local count = select("#", ...)
    local t, position = ...
    check_table("remove", count, t)
    local n = getn(t)
    if position == nil then
      position = n
    else
      position = check_int("remove", 2, count, position)
    end
    if n <= 0 then
      return
    end
    setn(t, n - 1)
    local result = rawget(t, position)
    while position < n do
      rawset(t, position, rawget(t, position + 1))
      position = position + 1
    end
    rawset(t, n, nil)
    return result
  end

  table.foreachi = function(...)
    local count = select("#", ...)
    local t, f = ...
    check_table("foreachi", count, t)
    local n = getn(t)
    if type(f) ~= "function" then
      error("bad argument #2 to 'foreachi' (function expected, got " .. got(count, 2, f)
        .. ")", 0)
    end
    for i = 1, n do
      local result = f(i, rawget(t, i))
      if result ~= nil then
        return result
      end
    end
  end

  return getn
end
"##;

/// The `os` functions a quest keeps: the clock, no files and no processes.
const OS_KEPT: [&str; 4] = ["date", "time", "clock", "difftime"];

/// The base functions a quest does not get: `load` (Lua 5.1 only) and `loadfile` would read
/// code the dialect translator never sees.
const BASE_REMOVED: [&str; 2] = ["load", "loadfile"];

/// The library chain of `InitializeLua` (`questlua.cpp:728-786`), below the locale directory.
const LIBRARIES: [&str; 4] = [
    "settings.lua",
    "quest/questlib.lua",
    "translate.lua",
    "quest/locale.lua",
];

/// The registry key of the function a [`BRIDGED`] name calls with its name and arguments.
///
/// [`crate::manager`] sets it for one script run, inside a `Lua::scope`, and removes it after, so
/// a bridged name called outside a run fails with "no script is running".
pub const BRIDGE: &str = "quest bridge";

/// The API names that reach the game through [`BRIDGE`], in [`crate::api::NAMES`] order: the
/// dialog (`say`, `raw_script`, the skins and the images), the chat lines, and the lookups the
/// ported systems can answer.
pub const BRIDGED: [&str; 18] = [
    "game.get_event_flag",
    "set_skin",
    "setskin",
    "say",
    "chat",
    "cmdchat",
    "syschat",
    "setleftimage",
    "settopimage",
    "getnpcid",
    "raw_script",
    "number",
    "mob_name",
    "get_time",
    "get_global_time",
    "notice",
    "npc.getrace",
    "npc.get_race",
];

/// The registry key of [`PRELUDE`]'s `getn`.
const GETN: &str = "quest getn";

/// The music `add_bgm_info` gives a map (`CHARACTER_AddBGMInfo`, `char.cpp:1937`).
#[derive(Clone, Debug, PartialEq)]
pub struct Bgm {
    /// The file the client plays, up to its first zero byte.
    pub name: Vec<u8>,
    /// The volume, as the script passed it, or 0.02 without one.
    pub volume: f64,
}

/// A destination `add_goto_info` records for the GM command `/goto` (`cmd_gm.cpp:191`): the
/// `int`s legacy read, before `CHARACTER_AddGotoInfo` narrows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GotoInfo {
    /// The name `/goto` matches.
    pub name: Vec<u8>,
    /// The empire, 0 for any.
    pub empire: i32,
    /// The map.
    pub map_index: i32,
    /// The position on the map, in map units.
    pub x: i32,
    /// See [`GotoInfo::x`].
    pub y: i32,
}

/// An arena `arena.add_map` records (`questlua_arena.cpp:52`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arena {
    /// The map.
    pub map_index: i32,
    /// Where the first duelist starts.
    pub start_a: (i32, i32),
    /// Where the second duelist starts.
    pub start_b: (i32, i32),
}

/// What the libraries record at load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// The music of each map; a later call for a map replaces the earlier one.
    pub bgm: BTreeMap<u32, Bgm>,
    /// Whether the client gets the volume with the music (`set_bgm_volume_enable`).
    pub bgm_volume_enable: bool,
    /// The `/goto` destinations, in order.
    pub goto_info: Vec<GotoInfo>,
    /// The arenas, in order.
    pub arenas: Vec<Arena>,
}

/// A quest the host numbered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quest {
    /// Its name.
    pub name: Vec<u8>,
    /// The number of each of its states.
    pub states: BTreeMap<Vec<u8>, i32>,
}

/// A script [`crate::qc`] refused, which the host leaves out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    /// The script, relative to the quest directory.
    pub path: PathBuf,
    /// Why.
    pub error: QcError,
}

/// Why the host could not load.
#[derive(Debug)]
pub enum HostError {
    /// A file could not be read.
    Io(PathBuf, io::Error),
    /// A library or a state table failed.
    Lua(mlua::Error),
    /// Two scripts compile the same `object/` file differently.
    Clash(Vec<u8>),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Io(path, error) => write!(f, "{}: {error}", path.display()),
            HostError::Lua(error) => write!(f, "{error}"),
            HostError::Clash(path) => write!(
                f,
                "two scripts compile object/{} differently",
                String::from_utf8_lossy(path)
            ),
        }
    }
}

impl std::error::Error for HostError {}

impl From<mlua::Error> for HostError {
    fn from(error: mlua::Error) -> Self {
        HostError::Lua(error)
    }
}

/// The quest Lua state with every script loaded.
pub struct Host {
    lua: Lua,
    quests: Vec<Quest>,
    objects: Objects,
    chunks: BTreeMap<Vec<u8>, Function>,
    refused: Vec<Refused>,
}

impl fmt::Debug for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Host")
            .field("quests", &self.quests.len())
            .field("objects", &self.objects.len())
            .field("refused", &self.refused)
            .finish_non_exhaustive()
    }
}

impl Host {
    /// Builds the state for the locale directory `locale_dir` (`locale/europe` in the Game
    /// data), which holds `settings.lua`, `translate.lua` and `quest/`.
    ///
    /// # Errors
    ///
    /// A library or state table fails, a file cannot be read, or two scripts clash.
    pub fn load(locale_dir: &Path) -> Result<Host, HostError> {
        let lua = new_state(locale_dir)?;
        for library in LIBRARIES {
            run_file(&lua, &locale_dir.join(library))?;
        }
        let quest_dir = locale_dir.join("quest");
        let order = sources::load_order(&quest_dir)
            .map_err(|error| HostError::Io(quest_dir.clone(), error))?;
        let qc = Compiler::new();
        let mut names: Vec<Vec<u8>> = Vec::new();
        let mut objects = Objects::new();
        let mut refused = Vec::new();
        for path in order {
            let full = quest_dir.join(&path);
            let source = fs::read(&full).map_err(|error| HostError::Io(full, error))?;
            let compiled = match qc.compile(&source) {
                Ok(compiled) => compiled,
                Err(error) => {
                    tracing::warn!(
                        script = %path.display(),
                        %error,
                        "qc refuses a quest script; it is not loaded"
                    );
                    refused.push(Refused { path, error });
                    continue;
                }
            };
            if !names.contains(&compiled.quest) {
                names.push(compiled.quest);
            }
            for (file, bytes) in compiled.files {
                match objects.get(&file) {
                    Some(existing) if *existing != bytes => return Err(HostError::Clash(file)),
                    _ => {
                        objects.insert(file, bytes);
                    }
                }
            }
        }
        let mut quests = Vec::with_capacity(names.len());
        for name in names {
            let file = [&b"state/"[..], &name].concat();
            let table = &objects[&file];
            load_chunk(&lua, table, &chunk_name(&file))?.call::<()>(())?;
            let states = index_states(&lua, &name)?;
            quests.push(Quest { name, states });
        }
        let mut chunks = BTreeMap::new();
        for (file, bytes) in &objects {
            if let Some(code) = runnable(file, bytes) {
                chunks.insert(file.clone(), load_chunk(&lua, &code, &chunk_name(file))?);
            }
        }
        Ok(Host {
            lua,
            quests,
            objects,
            chunks,
            refused,
        })
    }

    /// The Lua state.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// The quests, the quest numbered `n` at `n - 1`.
    pub fn quests(&self) -> &[Quest] {
        &self.quests
    }

    /// The number of a quest, from 1, as `RegisterQuest` gave it.
    pub fn quest_index(&self, name: &[u8]) -> Option<u32> {
        let position = self.quests.iter().position(|quest| quest.name == name)?;
        u32::try_from(position + 1).ok()
    }

    /// Every compiled file, keyed by its path below `object/`.
    pub fn objects(&self) -> &Objects {
        &self.objects
    }

    /// The compiled function of an `object/` file: a `when` script, a condition (a `.when` file
    /// that is not empty or a `begin_condition`) or a chat menu's `.arg` as `return ARG`.
    pub fn chunk(&self, file: &[u8]) -> Option<&Function> {
        self.chunks.get(file)
    }

    /// The scripts `qc` refused.
    pub fn refused(&self) -> &[Refused] {
        &self.refused
    }

    /// What the libraries recorded at load.
    pub fn settings(&self) -> Settings {
        self.lua
            .app_data_ref::<Settings>()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }
}

/// The source of the chunk an `object/` file runs as, or `None` for the state tables (run at
/// load), an empty `.when` (always true) and the `.arg` of an event other than `chat`, which
/// legacy compares as text.
fn runnable(file: &[u8], bytes: &[u8]) -> Option<Vec<u8>> {
    if file.starts_with(b"state/") {
        return None;
    }
    if file.ends_with(b".when") && bytes.is_empty() {
        return None;
    }
    if file.ends_with(b".arg") {
        if !file.windows(6).any(|window| window == b"/chat/") {
            return None;
        }
        // `ScriptToString` (`questlua.cpp:33`) runs `return ` and the file's first line.
        let line = bytes
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        return Some([&b"return "[..], line].concat());
    }
    Some(bytes.to_vec())
}

/// The name a chunk of `object/` reports its errors under.
fn chunk_name(file: &[u8]) -> String {
    format!("@object/{}", String::from_utf8_lossy(file))
}

/// Translates a chunk of the dialect and compiles it as Lua 5.1 text.
///
/// # Errors
///
/// The translator or Lua 5.1 refuses the chunk.
pub fn load_chunk(lua: &Lua, source: &[u8], name: &str) -> mlua::Result<Function> {
    let translated = dialect::translate(source).map_err(|error| mlua::Error::SyntaxError {
        message: format!("{name}: {error}"),
        incomplete_input: false,
    })?;
    lua.load(&translated)
        .set_name(name)
        .set_mode(ChunkMode::Text)
        .into_function()
}

fn run_file(lua: &Lua, path: &Path) -> Result<(), HostError> {
    let source = fs::read(path).map_err(|error| HostError::Io(path.to_path_buf(), error))?;
    let name = format!("@{}", path.display());
    load_chunk(lua, &source, &name)?.call::<()>(())?;
    Ok(())
}

/// Adds `table[number] = name` for every state of the quest's table
/// (`BuildStateIndexToName`), and returns the states.
fn index_states(lua: &Lua, quest: &[u8]) -> mlua::Result<BTreeMap<Vec<u8>, i32>> {
    let name = lua.create_string(quest)?;
    let Value::Table(table) = lua.globals().raw_get::<Value>(name)? else {
        return Err(mlua::Error::runtime(format!(
            "QUEST wrong quest state file for quest {}",
            String::from_utf8_lossy(quest)
        )));
    };
    let mut pairs = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key_is_string = matches!(key, Value::String(_) | Value::Integer(_) | Value::Number(_));
        let value_is_number = match &value {
            Value::Integer(_) | Value::Number(_) => true,
            Value::String(_) => lua.coerce_number(value.clone())?.is_some(),
            _ => false,
        };
        if key_is_string && value_is_number {
            pairs.push((key, value));
        }
    }
    let mut states = BTreeMap::new();
    for (key, value) in pairs {
        if let (Value::String(state), Value::Integer(number)) = (&key, &value) {
            if let Ok(number) = i32::try_from(*number) {
                states.insert(state.as_bytes().to_vec(), number);
            }
        }
        table.raw_set(value, key)?;
    }
    Ok(states)
}

fn new_state(locale_dir: &Path) -> Result<Lua, HostError> {
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::OS,
        LuaOptions::default(),
    )?;
    lua.set_app_data(Settings::default());
    let globals = lua.globals();
    let os: Table = globals.get("os")?;
    let kept = lua.create_table()?;
    for name in OS_KEPT {
        kept.set(name, os.get::<Value>(name)?)?;
    }
    globals.set("os", kept)?;
    for name in BASE_REMOVED {
        globals.set(name, Value::Nil)?;
    }
    register_api(&lua, locale_dir)?;
    install_loaders(&lua, locale_dir)?;
    let getn: Function = lua
        .load(PRELUDE)
        .set_name("=prelude")
        .set_mode(ChunkMode::Text)
        .eval()?;
    lua.set_named_registry_value(GETN, getn)?;
    Ok(lua)
}

/// `luaL_getn` of Lua 5.0 (`lauxlib.c`): the table's `n` field, else the size `setn` or `insert`
/// recorded, else a count up to the first nil.
///
/// # Errors
///
/// The state was not built by [`Host::load`].
pub fn getn(lua: &Lua, table: &Table) -> mlua::Result<i32> {
    let getn: Function = lua.named_registry_value(GETN)?;
    let n: Value = getn.call(table)?;
    c_int(lua, n)
}

/// Registers every legacy API name, as `AddLuaFunctionTable` did.
fn register_api(lua: &Lua, locale_dir: &Path) -> mlua::Result<()> {
    let globals = lua.globals();
    for name in api::NAMES {
        let function = match name {
            "add_bgm_info" => lua.create_function(add_bgm_info)?,
            "add_goto_info" => lua.create_function(add_goto_info)?,
            "set_bgm_volume_enable" => lua.create_function(|lua, _: MultiValue| {
                set_bgm_volume_enable(lua);
                Ok(())
            })?,
            "arena.add_map" => lua.create_function(arena_add_map)?,
            "get_locale_base_path" => {
                let path = lua.create_string(locale_dir.as_os_str().as_bytes())?;
                lua.create_function(move |_, ()| Ok(path.clone()))?
            }
            "q.yield" => lua
                .globals()
                .get::<Table>("coroutine")?
                .get::<Function>("yield")?,
            name if BRIDGED.contains(&name) => bridged(lua, name)?,
            _ => not_ported(lua, name)?,
        };
        match name.split_once('.') {
            None => globals.set(name, function)?,
            Some((namespace, member)) => {
                let table = if let Value::Table(table) = globals.get::<Value>(namespace)? {
                    table
                } else {
                    let table = lua.create_table()?;
                    globals.set(namespace, &table)?;
                    table
                };
                table.set(member, function)?;
            }
        }
    }
    Ok(())
}

/// A function whose system the Rewrite has not ported: calling it ends the script with an
/// error, as any legacy script error does.
fn not_ported(lua: &Lua, name: &'static str) -> mlua::Result<Function> {
    lua.create_function(move |_, _: MultiValue| -> mlua::Result<()> {
        Err(mlua::Error::runtime(format!("not ported: {name}")))
    })
}

/// A [`BRIDGED`] name: it hands its name and arguments to [`BRIDGE`], which the running script's
/// manager set.
fn bridged(lua: &Lua, name: &'static str) -> mlua::Result<Function> {
    lua.create_function(move |lua, args: MultiValue| {
        match lua.named_registry_value::<Option<Function>>(BRIDGE)? {
            Some(bridge) => bridge.call::<MultiValue>((name, args)),
            None => Err(mlua::Error::runtime(format!(
                "{name}: no script is running"
            ))),
        }
    })
}

/// `dofile`, `loadstring` and `print`, each reading through the dialect.
fn install_loaders(lua: &Lua, locale_dir: &Path) -> mlua::Result<()> {
    let globals = lua.globals();
    let root = locale_dir.to_path_buf();
    globals.set(
        "dofile",
        lua.create_function(move |lua, name: mlua::String| {
            let path = contained(&root, &name.as_bytes())?;
            let source = fs::read(&path).map_err(|error| {
                mlua::Error::runtime(format!("cannot read {}: {error}", path.display()))
            })?;
            load_chunk(lua, &source, &format!("@{}", path.display()))?.call::<MultiValue>(())
        })?,
    )?;
    globals.set(
        "loadstring",
        lua.create_function(
            |lua, (source, name): (mlua::String, Option<mlua::String>)| {
                let source = source.as_bytes().to_vec();
                let name = name.map_or_else(
                    || String::from_utf8_lossy(&source).into_owned(),
                    |name| String::from_utf8_lossy(&name.as_bytes()).into_owned(),
                );
                match load_chunk(lua, &source, &name) {
                    Ok(function) => Ok((Value::Function(function), Value::Nil)),
                    Err(error) => Ok((
                        Value::Nil,
                        Value::String(lua.create_string(error.to_string())?),
                    )),
                }
            },
        )?,
    )?;
    globals.set(
        "print",
        lua.create_function(|lua, values: MultiValue| {
            let tostring: Function = lua.globals().get("tostring")?;
            let mut line = Vec::new();
            for (index, value) in values.into_iter().enumerate() {
                if index > 0 {
                    line.push(b'\t');
                }
                line.extend_from_slice(&tostring.call::<mlua::String>(value)?.as_bytes());
            }
            tracing::info!(target: "quest", "{}", String::from_utf8_lossy(&line));
            Ok(())
        })?,
    )?;
    Ok(())
}

/// The file `dofile` names, which must lie below the locale directory.
fn contained(root: &Path, name: &[u8]) -> mlua::Result<PathBuf> {
    let path = Path::new(OsStr::from_bytes(name));
    let inside = path.strip_prefix(root).is_ok_and(|rest| {
        rest.components().next().is_some()
            && rest
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
    });
    if !inside {
        return Err(mlua::Error::runtime(format!(
            "dofile reads only below {}: {}",
            root.display(),
            path.display()
        )));
    }
    Ok(path.to_path_buf())
}

/// `(int)lua_tonumber(L, n)`: 0 for a value that is not a number, the fraction dropped toward
/// zero, and `INT_MIN` where x86's conversion has no `int` (out of range or NaN).
pub(crate) fn c_int(lua: &Lua, value: Value) -> mlua::Result<i32> {
    let Some(number) = lua.coerce_number(value)? else {
        return Ok(0);
    };
    let truncated = number.trunc();
    if !(-2_147_483_648.0..2_147_483_648.0).contains(&truncated) {
        return Ok(i32::MIN);
    }
    Ok(format!("{truncated:.0}").parse().unwrap_or(i32::MIN))
}

/// `lua_tostring(L, n)` up to its first zero byte, as a `const char*` reads it.
pub(crate) fn c_string(lua: &Lua, value: Value) -> mlua::Result<Option<Vec<u8>>> {
    Ok(lua.coerce_string(value)?.map(|text| {
        let bytes = text.as_bytes();
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        bytes[..end].to_vec()
    }))
}

/// `_add_bgm_info` (`questlua_global.cpp:413`): a map number and a string, else nothing.
fn add_bgm_info(lua: &Lua, (map, name, volume): (Value, Value, Value)) -> mlua::Result<()> {
    let map_is_number = matches!(map, Value::Integer(_) | Value::Number(_))
        || lua.coerce_number(map.clone())?.is_some();
    let name_is_string = matches!(
        name,
        Value::String(_) | Value::Integer(_) | Value::Number(_)
    );
    if !map_is_number || !name_is_string {
        return Ok(());
    }
    let map_index = u32::from_le_bytes(c_int(lua, map)?.to_le_bytes());
    let Some(name) = c_string(lua, name)? else {
        return Ok(());
    };
    let volume = lua.coerce_number(volume)?.unwrap_or(1.0 / 5.0 * 0.1);
    if let Some(mut settings) = lua.app_data_mut::<Settings>() {
        settings.bgm.insert(map_index, Bgm { name, volume });
    }
    Ok(())
}

/// `_set_bgm_volume_enable` (`questlua_global.cpp:406`).
fn set_bgm_volume_enable(lua: &Lua) {
    if let Some(mut settings) = lua.app_data_mut::<Settings>() {
        settings.bgm_volume_enable = true;
    }
}

/// `_add_goto_info` (`questlua_global.cpp:433`): nothing without a name.
fn add_goto_info(
    lua: &Lua,
    (name, empire, map, x, y): (Value, Value, Value, Value, Value),
) -> mlua::Result<()> {
    let name = c_string(lua, name)?;
    let info = (
        c_int(lua, empire)?,
        c_int(lua, map)?,
        c_int(lua, x)?,
        c_int(lua, y)?,
    );
    let Some(name) = name else {
        return Ok(());
    };
    if let Some(mut settings) = lua.app_data_mut::<Settings>() {
        settings.goto_info.push(GotoInfo {
            name,
            empire: info.0,
            map_index: info.1,
            x: info.2,
            y: info.3,
        });
    }
    Ok(())
}

/// `arena_add_map` (`questlua_arena.cpp:52`).
fn arena_add_map(
    lua: &Lua,
    (map, ax, ay, bx, by): (Value, Value, Value, Value, Value),
) -> mlua::Result<()> {
    let arena = Arena {
        map_index: c_int(lua, map)?,
        start_a: (c_int(lua, ax)?, c_int(lua, ay)?),
        start_b: (c_int(lua, bx)?, c_int(lua, by)?),
    };
    if let Some(mut settings) = lua.app_data_mut::<Settings>() {
        settings.arenas.push(arena);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prelude() -> Lua {
        let lua = Lua::new();
        lua.load(PRELUDE).exec().unwrap();
        lua
    }

    fn eval<T: mlua::FromLuaMulti>(lua: &Lua, code: &str) -> T {
        lua.load(code)
            .eval()
            .unwrap_or_else(|error| panic!("{code}: {error}"))
    }

    fn error(lua: &Lua, code: &str) -> String {
        lua.load(code).exec().unwrap_err().to_string()
    }

    #[test]
    fn getn_reads_n_then_the_recorded_size_then_counts() {
        let lua = prelude();
        assert_eq!(eval::<i32>(&lua, "return table.getn({n = 3})"), 3);
        assert_eq!(eval::<i32>(&lua, "return table.getn({n = 0, 1, 2})"), 0);
        // A count stops at the first nil, where Lua 5.1's `#` may not.
        assert_eq!(eval::<i32>(&lua, "return table.getn({1, 2, nil, 4})"), 2);
        assert_eq!(eval::<i32>(&lua, "return table.getn({n = 'x', 1})"), 1);
        assert_eq!(eval::<i32>(&lua, "return table.getn({n = -1, 1, 2})"), 2);
        assert_eq!(eval::<i32>(&lua, "return table.getn({n = '2.9', 1})"), 2);
        // An `n` no `int` holds reads as `INT_MIN`, so the count decides.
        assert_eq!(eval::<i32>(&lua, "return table.getn({n = 1e10, 1, 2})"), 2);
        assert_eq!(eval::<i32>(&lua, "return table.getn({n = 0/0, 1})"), 1);
        assert_eq!(
            eval::<i32>(&lua, "return table.getn({n = 2147483647.9, 1})"),
            i32::MAX
        );
        assert_eq!(
            eval::<i32>(&lua, "return table.getn({n = -2147483648.5})"),
            0
        );
        assert_eq!(
            eval::<i32>(&lua, "local t = {} table.setn(t, 5) return table.getn(t)"),
            5
        );
        assert!(eval::<bool>(
            &lua,
            "local t = {} table.setn(t, 5) return rawget(t, 'n') == nil"
        ));
        assert_eq!(
            eval::<i32>(&lua, "local t = {n = 0} table.setn(t, 5) return t.n"),
            5
        );
        assert!(error(&lua, "table.getn()")
            .contains("bad argument #1 to 'getn' (table expected, got no value)"));
        assert!(error(&lua, "table.getn(nil)")
            .contains("bad argument #1 to 'getn' (table expected, got nil)"));
        assert!(error(&lua, "table.setn({})")
            .contains("bad argument #2 to 'setn' (number expected, got no value)"));
    }

    #[test]
    fn insert_and_remove_keep_the_size_as_lua_5_0_did() {
        let lua = prelude();
        assert_eq!(
            eval::<String>(
                &lua,
                "local t = {} table.insert(t, 'a') table.insert(t, 1, 'b') \
                 table.insert(t, 2, 'c') return table.concat(t, ',') .. table.getn(t)"
            ),
            "b,c,a3"
        );
        // Past the end, the size grows to the position.
        assert_eq!(
            eval::<(i32, String)>(
                &lua,
                "local t = {} table.insert(t, 5, 'x') return table.getn(t), t[5]"
            ),
            (5, "x".to_owned())
        );
        // Two arguments insert the second, even a nil, at the end.
        assert_eq!(
            eval::<i32>(
                &lua,
                "local t = {} table.insert(t, nil) return table.getn(t)"
            ),
            1
        );
        assert_eq!(
            eval::<i32>(&lua, "local t = {n = 2} table.insert(t, 'x') return t.n"),
            3
        );
        assert!(error(&lua, "table.insert({})")
            .contains("bad argument #2 to 'insert' (number expected, got no value)"));
        assert!(error(&lua, "table.insert({}, 'a', 1)")
            .contains("bad argument #2 to 'insert' (number expected, got string)"));
        assert_eq!(
            eval::<(String, String, i32)>(
                &lua,
                "local t = {'a', 'b', 'c'} local last = table.remove(t) \
                 local first = table.remove(t, 1) return last, first .. t[1], table.getn(t)"
            ),
            ("c".to_owned(), "ab".to_owned(), 1)
        );
        assert_eq!(eval::<i32>(&lua, "return select('#', table.remove({}))"), 0);
        assert_eq!(
            eval::<i32>(&lua, "return select('#', table.remove({}, nil))"),
            0
        );
        assert!(error(&lua, "table.remove({}, 'x')")
            .contains("bad argument #2 to 'remove' (number expected, got string)"));
        // The size recorded by `insert` bounds `remove`, not the nils.
        assert_eq!(
            eval::<(i32, bool)>(
                &lua,
                "local t = {} table.insert(t, 3, 'x') local v = table.remove(t) \
                 return table.getn(t), v == 'x'"
            ),
            (2, true)
        );
    }

    #[test]
    fn foreachi_walks_the_size_and_stops_at_a_result() {
        let lua = prelude();
        assert_eq!(
            eval::<String>(
                &lua,
                "local seen = '' table.foreachi({n = 3, 'a', 'b'}, function(i, v) \
                 seen = seen .. i .. tostring(v) end) return seen"
            ),
            "1a2b3nil"
        );
        assert_eq!(
            eval::<i32>(
                &lua,
                "return table.foreachi({5, 6, 7}, function(i, v) if v == 6 then return i end end)"
            ),
            2
        );
        assert!(error(&lua, "table.foreachi({})")
            .contains("bad argument #2 to 'foreachi' (function expected, got no value)"));
    }

    #[test]
    fn a_generic_for_over_a_table_walks_it_with_next() {
        let lua = prelude();
        let code = dialect::translate(
            b"local t = {a = 1, b = 2} local sum = 0 for k, v in t do sum = sum + v end \
              for k, v in pairs(t) do sum = sum + v end return sum",
        )
        .unwrap();
        assert_eq!(lua.load(&code).eval::<i32>().unwrap(), 6);
        assert_eq!(
            eval::<i32>(
                &lua,
                "local n = 0 for i, v in __compat_iter(ipairs({4, 5})) do n = n + i end return n"
            ),
            3
        );
    }

    fn recorders() -> Lua {
        let lua = Lua::new();
        lua.set_app_data(Settings::default());
        let globals = lua.globals();
        globals
            .set("add_bgm_info", lua.create_function(add_bgm_info).unwrap())
            .unwrap();
        globals
            .set("add_goto_info", lua.create_function(add_goto_info).unwrap())
            .unwrap();
        globals
            .set("arena_add_map", lua.create_function(arena_add_map).unwrap())
            .unwrap();
        globals
            .set(
                "set_bgm_volume_enable",
                lua.create_function(|lua, _: MultiValue| {
                    set_bgm_volume_enable(lua);
                    Ok(())
                })
                .unwrap(),
            )
            .unwrap();
        lua
    }

    #[test]
    fn the_recorders_read_their_arguments_as_legacy_did() {
        let lua = recorders();
        lua.load(
            "add_bgm_info('7', 'a.mp3') add_bgm_info(7, 'b.mp3', 0.5) \
             add_bgm_info('x', 'c.mp3') add_bgm_info(8, {}) add_bgm_info(9, 12) \
             add_bgm_info(-1, 'a\\0b') \
             add_goto_info(nil, 1, 2, 3, 4) add_goto_info('g', -1.9, 2.9, '12', {}) \
             add_goto_info(5, 1e10, 0/0, -1e10, 2147483647.5) \
             arena_add_map(1, 2, 3, 4, 5) set_bgm_volume_enable()",
        )
        .exec()
        .unwrap();
        let settings = lua.app_data_ref::<Settings>().unwrap().clone();
        let bgm: Vec<(u32, &[u8], f64)> = settings
            .bgm
            .iter()
            .map(|(map, bgm)| (*map, &bgm.name[..], bgm.volume))
            .collect();
        assert_eq!(
            bgm,
            [
                (7, &b"b.mp3"[..], 0.5),
                (9, &b"12"[..], 1.0 / 5.0 * 0.1),
                (u32::MAX, &b"a"[..], 1.0 / 5.0 * 0.1),
            ]
        );
        let goto: Vec<(&[u8], i32, i32, i32, i32)> = settings
            .goto_info
            .iter()
            .map(|info| (&info.name[..], info.empire, info.map_index, info.x, info.y))
            .collect();
        assert_eq!(
            goto,
            [
                (&b"g"[..], -1, 2, 12, 0),
                (&b"5"[..], i32::MIN, i32::MIN, i32::MIN, i32::MAX),
            ]
        );
        assert_eq!(
            settings.arenas,
            [Arena {
                map_index: 1,
                start_a: (2, 3),
                start_b: (4, 5)
            }]
        );
        assert!(settings.bgm_volume_enable);
    }

    #[test]
    fn dofile_names_a_file_below_the_locale_directory() {
        let root = Path::new("locale/europe");
        assert!(contained(root, b"locale/europe/quest/questlib.lua").is_ok());
        assert!(contained(root, b"locale/europe/./settings.lua").is_ok());
        for outside in [
            &b"locale/europe"[..],
            b"locale/europe/",
            b"locale/europe/../x.lua",
            b"locale/europe/quest/../../x.lua",
            b"locale/europe2/x.lua",
            b"/etc/passwd",
            b"x.lua",
        ] {
            assert!(
                contained(root, outside).is_err(),
                "{}",
                String::from_utf8_lossy(outside)
            );
        }
    }
}
